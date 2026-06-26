//! Database access for Poopman, implemented CSP-style.
//!
//! The SQLite `Connection` is **owned by a single background thread**. Callers
//! never touch it directly and there is no `Mutex`; instead each operation is
//! sent to that thread as a job over a channel, and the result comes back over a
//! per-call reply channel. This is the "share memory by communicating" model —
//! the connection has exactly one owner, so data races are impossible by
//! construction and a panic inside one query can't poison a lock for the others.

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::types::{BodyType, Environment, EnvVar, HistoryItem, HttpMethod, RequestData};

/// A unit of work executed on the database's owning thread.
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Map a `history` row (id, timestamp, method, url, request_headers, request_body)
/// into a `HistoryItem`. Shared by `load_recent_history` and `search_history` so
/// the two queries can never drift in how they decode a row.
fn row_to_history_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    let id: i64 = row.get(0)?;
    let timestamp: String = row.get(1)?;
    let method: String = row.get(2)?;
    let url: String = row.get(3)?;
    let request_headers: String = row.get(4)?;
    let request_body: String = row.get(5)?;

    let headers: Vec<(String, String)> =
        serde_json::from_str(&request_headers).unwrap_or_default();
    let body: BodyType = serde_json::from_str(&request_body).unwrap_or_default();

    let request = RequestData {
        method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
        url,
        headers,
        body,
    };
    Ok(HistoryItem::new(id, timestamp, request, None))
}

/// Escape a user query so SQLite `LIKE` treats `%`, `_`, and `\` literally.
/// Paired with `ESCAPE '\'` in the SQL. Backslash must be escaped first.
fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Handle to the database thread. Cloneable senders make this cheap to share
/// (wrapped in `Arc` by the app); dropping every handle stops the thread.
pub struct Database {
    tx: Sender<Job>,
}

impl Database {
    /// Open (or create) the on-disk database and start its owning thread.
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;

        // Create directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        Self::init_schema(&conn)?;
        Ok(Self::spawn(conn))
    }

    /// Move an initialized connection onto its owning thread and return a handle.
    fn spawn(mut conn: Connection) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::spawn(move || {
            // Run jobs until every handle (and thus every Sender) is dropped, at
            // which point recv() errors and the thread exits cleanly.
            while let Ok(job) = rx.recv() {
                job(&mut conn);
            }
        });
        Self { tx }
    }

    /// Send `f` to the owning thread and block until it returns a result.
    fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Box::new(move |conn| {
                let _ = reply_tx.send(f(conn));
            }))
            .map_err(|_| anyhow!("database thread is not running"))?;
        reply_rx
            .recv()
            .map_err(|_| anyhow!("database thread dropped the response"))?
    }

    /// Create all tables + indexes if missing. Shared by the real DB and tests
    /// so the two schemas can never drift.
    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp TEXT NOT NULL,
                 method TEXT NOT NULL,
                 url TEXT NOT NULL,
                 request_headers TEXT,
                 request_body TEXT,
                 status_code INTEGER,
                 duration_ms INTEGER,
                 response_headers TEXT,
                 response_body TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_timestamp ON history(timestamp DESC);
             CREATE TABLE IF NOT EXISTS environments (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS env_variables (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS app_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Get the database file path
    fn get_db_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot find home directory"))?;
        Ok(home.join(".poopman").join("history.db"))
    }

    /// Insert a new history item (request only, no response - aligned with Postman)
    pub fn insert_history(
        &self,
        method: &str,
        url: &str,
        request_headers: &str,
        request_body: &BodyType,
    ) -> Result<i64> {
        let method = method.to_string();
        let url = url.to_string();
        let request_headers = request_headers.to_string();
        // Serialize body type to JSON before crossing the channel.
        let body_json = serde_json::to_string(request_body).unwrap_or_default();

        self.call(move |conn| {
            let timestamp = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO history (timestamp, method, url, request_headers, request_body)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![timestamp, method, url, request_headers, body_json],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Load recent history items (request only, no response - aligned with Postman)
    pub fn load_recent_history(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, method, url, request_headers, request_body
                 FROM history
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?1",
            )?;

            // rusqlite 0.40 dropped the `ToSql` impl for `usize`; bind as i64.
            let items = stmt.query_map([limit as i64], row_to_history_item)?;

            let mut result = Vec::new();
            for item in items {
                result.push(item?);
            }
            Ok(result)
        })
    }

    /// Search history by URL or method (case-insensitive substring), newest
    /// first, up to `limit` rows. An empty query matches everything.
    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryItem>> {
        let pattern = format!("%{}%", escape_like(query));
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, method, url, request_headers, request_body
                 FROM history
                 WHERE url LIKE ?1 ESCAPE '\\' OR method LIKE ?1 ESCAPE '\\'
                 ORDER BY timestamp DESC, id DESC
                 LIMIT ?2",
            )?;
            let items = stmt.query_map(params![pattern, limit as i64], row_to_history_item)?;
            let mut result = Vec::new();
            for item in items {
                result.push(item?);
            }
            Ok(result)
        })
    }

    /// Delete a history item by ID
    #[allow(dead_code)]
    pub fn delete_history(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            conn.execute("DELETE FROM history WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    /// Clear all history
    pub fn clear_all_history(&self) -> Result<()> {
        self.call(|conn| {
            conn.execute("DELETE FROM history", [])?;
            Ok(())
        })
    }

    /// Get total history count
    #[allow(dead_code)]
    pub fn get_history_count(&self) -> Result<usize> {
        self.call(|conn| {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?;
            Ok(count as usize)
        })
    }

    // ===== Environments =====

    /// Load all environments (with their variables), ordered by position.
    pub fn load_environments(&self) -> Result<Vec<Environment>> {
        self.call(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, name FROM environments ORDER BY position, id")?;
            let env_rows: Vec<(i64, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut result = Vec::with_capacity(env_rows.len());
            for (id, name) in env_rows {
                let mut vstmt = conn.prepare(
                    "SELECT enabled, key, value FROM env_variables
                     WHERE environment_id = ?1 ORDER BY position, id",
                )?;
                let variables = vstmt
                    .query_map([id], |row| {
                        Ok(EnvVar {
                            enabled: row.get::<_, i64>(0)? != 0,
                            key: row.get(1)?,
                            value: row.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                result.push(Environment { id, name, variables });
            }
            Ok(result)
        })
    }

    /// Create a new (empty) environment, returning its id.
    pub fn create_environment(&self, name: &str) -> Result<i64> {
        let name = name.to_string();
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO environments (name, position)
                 VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM environments))",
                params![name],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn rename_environment(&self, id: i64, name: &str) -> Result<()> {
        let name = name.to_string();
        self.call(move |conn| {
            conn.execute("UPDATE environments SET name = ?1 WHERE id = ?2", params![name, id])?;
            Ok(())
        })
    }

    pub fn delete_environment(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            // env_variables rows are removed by ON DELETE CASCADE (foreign_keys = ON).
            conn.execute("DELETE FROM environments WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    /// Replace all variables of an environment in a single transaction.
    pub fn replace_variables(&self, environment_id: i64, vars: &[EnvVar]) -> Result<()> {
        let vars = vars.to_vec();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "DELETE FROM env_variables WHERE environment_id = ?1",
                params![environment_id],
            )?;
            for (position, v) in vars.iter().enumerate() {
                tx.execute(
                    "INSERT INTO env_variables (environment_id, enabled, key, value, position)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![environment_id, v.enabled as i64, v.key, v.value, position as i64],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Active environment id, or None for "No Environment".
    pub fn get_active_environment_id(&self) -> Result<Option<i64>> {
        self.call(|conn| {
            let value: Option<String> = conn
                .query_row(
                    "SELECT value FROM app_meta WHERE key = 'active_environment_id'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(value.and_then(|s| s.parse::<i64>().ok()))
        })
    }

    pub fn set_active_environment_id(&self, id: Option<i64>) -> Result<()> {
        self.call(move |conn| {
            match id {
                Some(id) => {
                    conn.execute(
                        "INSERT INTO app_meta (key, value) VALUES ('active_environment_id', ?1)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![id.to_string()],
                    )?;
                }
                None => {
                    conn.execute("DELETE FROM app_meta WHERE key = 'active_environment_id'", [])?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        Database::init_schema(&conn).unwrap();
        Database::spawn(conn)
    }

    #[test]
    fn crud_and_active() {
        let db = mem_db();
        let id = db.create_environment("dev").unwrap();
        db.replace_variables(
            id,
            &[
                EnvVar { enabled: true, key: "baseUrl".into(), value: "http://x".into() },
                EnvVar { enabled: false, key: "token".into(), value: "abc".into() },
            ],
        )
        .unwrap();

        let envs = db.load_environments().unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "dev");
        assert_eq!(envs[0].variables.len(), 2);
        assert_eq!(envs[0].variables[0].key, "baseUrl");
        assert!(!envs[0].variables[1].enabled);

        db.rename_environment(id, "staging").unwrap();
        assert_eq!(db.load_environments().unwrap()[0].name, "staging");

        assert_eq!(db.get_active_environment_id().unwrap(), None);
        db.set_active_environment_id(Some(id)).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), Some(id));
        db.set_active_environment_id(None).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), None);

        db.delete_environment(id).unwrap();
        assert!(db.load_environments().unwrap().is_empty());
    }

    #[test]
    fn history_roundtrip() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/x", "[]", &crate::types::BodyType::None)
            .unwrap();
        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request.url, "https://api.test/x");
        db.clear_all_history().unwrap();
        assert!(db.load_recent_history(10).unwrap().is_empty());
    }

    #[test]
    fn search_history_matches_url_and_method_newest_first() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/users", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("POST", "https://api.test/login", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("DELETE", "https://api.test/orders/1", "[]", &crate::types::BodyType::None)
            .unwrap();

        // URL substring
        let r = db.search_history("login", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.url, "https://api.test/login");

        // method match, case-insensitive
        let r = db.search_history("post", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.method, HttpMethod::POST);

        // shared substring across all three, newest (last inserted) first
        let r = db.search_history("api.test", 10).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].request.url, "https://api.test/orders/1");
    }

    #[test]
    fn search_history_escapes_wildcards() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/a%b", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("GET", "https://api.test/a_b", "[]", &crate::types::BodyType::None)
            .unwrap();
        db.insert_history("GET", "https://api.test/axb", "[]", &crate::types::BodyType::None)
            .unwrap();

        // '%' must be treated literally: matches only the URL with a literal '%'
        let r = db.search_history("a%b", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.url, "https://api.test/a%b");

        // '_' must be treated literally: matches only the URL with a literal '_',
        // not the single-char wildcard that would also match "/axb" and "/a%b".
        let r = db.search_history("a_b", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].request.url, "https://api.test/a_b");
    }

    #[test]
    fn search_history_empty_query_matches_all() {
        let db = mem_db();
        db.insert_history("GET", "https://api.test/users", "[]", &crate::types::BodyType::None)
            .unwrap();
        let r = db.search_history("", 10).unwrap();
        assert_eq!(r.len(), 1);
    }
}
