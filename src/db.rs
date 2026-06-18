use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::types::{Environment, EnvVar, HistoryItem, HttpMethod, RequestData};

/// Database manager for Poopman
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Create a new database connection
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;

        // Create directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;

        // Foreign keys are off per-connection by default in SQLite.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // Initialize schema
        conn.execute(
            "CREATE TABLE IF NOT EXISTS history (
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
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON history(timestamp DESC)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS environments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS env_variables (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                enabled INTEGER NOT NULL DEFAULT 1,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                position INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Get the database file path
    fn get_db_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        Ok(home.join(".poopman").join("history.db"))
    }

    /// Insert a new history item (request only, no response - aligned with Postman)
    pub fn insert_history(
        &self,
        method: &str,
        url: &str,
        request_headers: &str,
        request_body: &crate::types::BodyType,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();

        let timestamp = chrono::Utc::now().to_rfc3339();

        // Serialize body type to JSON
        let body_json = serde_json::to_string(request_body).unwrap_or_default();

        conn.execute(
            "INSERT INTO history (timestamp, method, url, request_headers, request_body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                timestamp,
                method,
                url,
                request_headers,
                body_json,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Load recent history items (request only, no response - aligned with Postman)
    pub fn load_recent_history(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, url, request_headers, request_body
             FROM history
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;

        // rusqlite 0.40 dropped the `ToSql` impl for `usize`; bind as i64.
        let items = stmt.query_map([limit as i64], |row| {
            let id: i64 = row.get(0)?;
            let timestamp: String = row.get(1)?;
            let method: String = row.get(2)?;
            let url: String = row.get(3)?;
            let request_headers: String = row.get(4)?;
            let request_body: String = row.get(5)?;

            let headers: Vec<(String, String)> =
                serde_json::from_str(&request_headers).unwrap_or_default();

            // Deserialize body type from JSON, fallback to default if fails
            let body: crate::types::BodyType =
                serde_json::from_str(&request_body).unwrap_or_default();

            let request = RequestData {
                method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
                url,
                headers,
                body,
            };

            Ok(HistoryItem::new(id, timestamp, request, None))
        })?;

        let mut result = Vec::new();
        for item in items {
            result.push(item?);
        }

        Ok(result)
    }

    /// Delete a history item by ID
    #[allow(dead_code)]
    pub fn delete_history(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Clear all history
    pub fn clear_all_history(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Get total history count
    #[allow(dead_code)]
    pub fn get_history_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    // ===== Environments =====

    /// Load all environments (with their variables), ordered by position.
    pub fn load_environments(&self) -> Result<Vec<Environment>> {
        let conn = self.conn.lock().unwrap();

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
    }

    /// Create a new (empty) environment, returning its id.
    pub fn create_environment(&self, name: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO environments (name, position)
             VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM environments))",
            params![name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn rename_environment(&self, id: i64, name: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE environments SET name = ?1 WHERE id = ?2", params![name, id])?;
        Ok(())
    }

    pub fn delete_environment(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // env_variables rows are removed by ON DELETE CASCADE (foreign_keys = ON).
        conn.execute("DELETE FROM environments WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Replace all variables of an environment in a single transaction.
    pub fn replace_variables(&self, environment_id: i64, vars: &[EnvVar]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
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
    }

    /// Active environment id, or None for "No Environment".
    pub fn get_active_environment_id(&self) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'active_environment_id'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|s| s.parse::<i64>().ok()))
    }

    pub fn set_active_environment_id(&self, id: Option<i64>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute(
            "CREATE TABLE environments (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)",
            [],
        ).unwrap();
        conn.execute(
            "CREATE TABLE env_variables (id INTEGER PRIMARY KEY AUTOINCREMENT, environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE, enabled INTEGER NOT NULL DEFAULT 1, key TEXT NOT NULL, value TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0)",
            [],
        ).unwrap();
        conn.execute(
            "CREATE TABLE app_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        ).unwrap();
        Database { conn: Arc::new(Mutex::new(conn)) }
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
}
