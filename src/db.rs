//! Database access for Poopman, implemented CSP-style.
//!
//! The SQLite `Connection` is **owned by a single background thread**. Callers
//! never touch it directly and there is no `Mutex`; instead each operation is
//! sent to that thread as a job over a channel, and the result comes back over a
//! per-call reply channel. This is the "share memory by communicating" model —
//! the connection has exactly one owner, so data races are impossible by
//! construction and a panic inside one query can't poison a lock for the others.

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Sender},
};
use std::thread;

use crate::types::{
    AppSettings, AuthConfig, BodyType, Collection, CollectionFolder, EnvVar, Environment,
    HeaderState, HistoryItem, HttpMethod, ParamState, RequestData, SavedRequest,
};

/// A unit of work executed on the database's owning thread.
type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Map a `history` row (id, timestamp, method, url, request_headers,
/// request_body, request_auth) into a `HistoryItem`. Shared by
/// `load_recent_history` and `search_history` so the two queries can never
/// drift in how they decode a row.
fn row_to_history_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    let id: i64 = row.get(0)?;
    let timestamp: String = row.get(1)?;
    let method: String = row.get(2)?;
    let url: String = row.get(3)?;
    let request_headers: String = row.get(4)?;
    let request_body: String = row.get(5)?;
    let request_auth: Option<String> = row.get(6)?;

    let headers: Vec<(String, String)> = serde_json::from_str(&request_headers).unwrap_or_default();
    let body: BodyType = serde_json::from_str(&request_body).unwrap_or_default();
    let auth: crate::types::AuthConfig = request_auth
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let request = RequestData {
        method: HttpMethod::from_str(&method).unwrap_or(HttpMethod::GET),
        url,
        headers,
        body,
        auth,
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

#[derive(Debug, Clone)]
struct FolderRow {
    id: i64,
    collection_id: i64,
    parent_id: Option<i64>,
    name: String,
    position: i64,
}

#[derive(Debug, Clone)]
struct SavedRequestRow {
    id: i64,
    collection_id: i64,
    folder_id: Option<i64>,
    name: String,
    request_json: String,
    params_json: String,
    headers_json: String,
    position: i64,
    created_at: String,
    updated_at: String,
}

fn decode_saved_request(row: SavedRequestRow) -> Result<SavedRequest> {
    Ok(SavedRequest {
        id: row.id,
        collection_id: row.collection_id,
        folder_id: row.folder_id,
        name: row.name,
        request: serde_json::from_str(&row.request_json)?,
        params_state: serde_json::from_str::<Vec<ParamState>>(&row.params_json)?,
        headers_state: serde_json::from_str::<Vec<HeaderState>>(&row.headers_json)?,
        position: row.position,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn build_folder(
    id: i64,
    folders: &HashMap<i64, FolderRow>,
    requests: &mut HashMap<(i64, Option<i64>), Vec<SavedRequest>>,
) -> Result<CollectionFolder> {
    let row = folders
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("missing collection folder {}", id))?;

    let mut child_ids: Vec<(i64, i64)> = folders
        .values()
        .filter(|child| child.collection_id == row.collection_id && child.parent_id == Some(id))
        .map(|child| (child.position, child.id))
        .collect();
    child_ids.sort_by_key(|(position, child_id)| (*position, *child_id));

    let child_folders = child_ids
        .into_iter()
        .map(|(_, child_id)| build_folder(child_id, folders, requests))
        .collect::<Result<Vec<_>>>()?;

    let mut child_requests = requests
        .remove(&(row.collection_id, Some(id)))
        .unwrap_or_default();
    child_requests.sort_by_key(|request| (request.position, request.id));

    Ok(CollectionFolder {
        id: row.id,
        collection_id: row.collection_id,
        parent_id: row.parent_id,
        name: row.name,
        position: row.position,
        folders: child_folders,
        requests: child_requests,
    })
}

fn build_collection_tree(
    collection_rows: Vec<(i64, String, i64)>,
    folder_rows: Vec<FolderRow>,
    request_rows: Vec<SavedRequestRow>,
) -> Result<Vec<Collection>> {
    let folders: HashMap<i64, FolderRow> = folder_rows
        .into_iter()
        .map(|folder| (folder.id, folder))
        .collect();
    let mut requests: HashMap<(i64, Option<i64>), Vec<SavedRequest>> = HashMap::new();
    for row in request_rows {
        let request = decode_saved_request(row)?;
        requests
            .entry((request.collection_id, request.folder_id))
            .or_default()
            .push(request);
    }

    let mut collections = Vec::with_capacity(collection_rows.len());
    for (id, name, position) in collection_rows {
        let mut root_folder_ids: Vec<(i64, i64)> = folders
            .values()
            .filter(|folder| folder.collection_id == id && folder.parent_id.is_none())
            .map(|folder| (folder.position, folder.id))
            .collect();
        root_folder_ids.sort_by_key(|(folder_position, folder_id)| (*folder_position, *folder_id));
        let collection_folders = root_folder_ids
            .into_iter()
            .map(|(_, folder_id)| build_folder(folder_id, &folders, &mut requests))
            .collect::<Result<Vec<_>>>()?;

        let mut collection_requests = requests.remove(&(id, None)).unwrap_or_default();
        collection_requests.sort_by_key(|request| (request.position, request.id));

        collections.push(Collection {
            id,
            name,
            position,
            folders: collection_folders,
            requests: collection_requests,
        });
    }
    collections.sort_by_key(|collection| (collection.position, collection.id));
    Ok(collections)
}

fn require_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("name cannot be empty"));
    }
    Ok(name.to_string())
}

fn copy_name(name: &str) -> String {
    format!("{} (Copy)", name)
}

fn validate_folder_parent(
    tx: &Transaction<'_>,
    collection_id: i64,
    folder_id: Option<i64>,
) -> Result<()> {
    if let Some(folder_id) = folder_id {
        let folder_collection: i64 = tx.query_row(
            "SELECT collection_id FROM collection_folders WHERE id = ?1",
            [folder_id],
            |row| row.get(0),
        )?;
        if folder_collection != collection_id {
            return Err(anyhow!(
                "folder {} does not belong to collection {}",
                folder_id,
                collection_id
            ));
        }
    }
    Ok(())
}

fn encode_saved_request(
    request: &RequestData,
    params_state: &[ParamState],
    headers_state: &[HeaderState],
) -> Result<(String, String, String)> {
    Ok((
        serde_json::to_string(request)?,
        serde_json::to_string(params_state)?,
        serde_json::to_string(headers_state)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn insert_saved_request_tx(
    tx: &Transaction<'_>,
    collection_id: i64,
    folder_id: Option<i64>,
    name: &str,
    request: &RequestData,
    params_state: &[ParamState],
    headers_state: &[HeaderState],
    position: Option<i64>,
) -> Result<i64> {
    validate_folder_parent(tx, collection_id, folder_id)?;
    let (request_json, params_json, headers_json) =
        encode_saved_request(request, params_state, headers_state)?;
    let position = match position {
        Some(position) => position,
        None => tx.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1
             FROM saved_requests
             WHERE collection_id = ?1 AND folder_id IS ?2",
            params![collection_id, folder_id],
            |row| row.get(0),
        )?,
    };
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO saved_requests
             (collection_id, folder_id, name, request_json, params_json, headers_json,
              position, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            collection_id,
            folder_id,
            name,
            request_json,
            params_json,
            headers_json,
            position,
            now,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

fn insert_folder_tree_tx(
    tx: &Transaction<'_>,
    collection_id: i64,
    parent_id: Option<i64>,
    folder: &CollectionFolder,
) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1
         FROM collection_folders
         WHERE collection_id = ?1 AND parent_id IS ?2",
        params![collection_id, parent_id],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO collection_folders (collection_id, parent_id, name, position)
         VALUES (?1, ?2, ?3, ?4)",
        params![collection_id, parent_id, folder.name, position],
    )?;
    let folder_id = tx.last_insert_rowid();

    for request in &folder.requests {
        insert_saved_request_tx(
            tx,
            collection_id,
            Some(folder_id),
            &request.name,
            &request.request,
            &request.params_state,
            &request.headers_state,
            None,
        )?;
    }
    for child in &folder.folders {
        insert_folder_tree_tx(tx, collection_id, Some(folder_id), child)?;
    }
    Ok(folder_id)
}

fn insert_collection_tree_tx(
    tx: &Transaction<'_>,
    collection: &Collection,
    name: &str,
) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM collections",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO collections (name, position) VALUES (?1, ?2)",
        params![name, position],
    )?;
    let collection_id = tx.last_insert_rowid();

    for request in &collection.requests {
        insert_saved_request_tx(
            tx,
            collection_id,
            None,
            &request.name,
            &request.request,
            &request.params_state,
            &request.headers_state,
            None,
        )?;
    }
    for folder in &collection.folders {
        insert_folder_tree_tx(tx, collection_id, None, folder)?;
    }
    Ok(collection_id)
}

fn find_folder(folders: &[CollectionFolder], id: i64) -> Option<&CollectionFolder> {
    for folder in folders {
        if folder.id == id {
            return Some(folder);
        }
        if let Some(found) = find_folder(&folder.folders, id) {
            return Some(found);
        }
    }
    None
}

/// Handle to the database thread. Cloneable senders make this cheap to share
/// (wrapped in `Arc` by the app); dropping every handle stops the thread.
pub struct Database {
    tx: Sender<Job>,
    ui_thread: OnceLock<thread::ThreadId>,
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
            #[cfg(feature = "profile")]
            profiling::register_thread!("database");

            // Run jobs until every handle (and thus every Sender) is dropped, at
            // which point recv() errors and the thread exits cleanly.
            while let Ok(job) = rx.recv() {
                #[cfg(feature = "profile")]
                profiling::scope!("database job");
                job(&mut conn);
            }
        });
        Self {
            tx,
            ui_thread: OnceLock::new(),
        }
    }

    /// Test-only: an in-memory database with the schema initialized. Shared by
    /// the db and app test suites so they exercise the same schema/migrations.
    #[cfg(test)]
    pub(crate) fn new_in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        Self::init_schema(&conn).expect("init schema");
        Self::spawn(conn)
    }

    /// Send `f` to the owning thread and block until it returns a result.
    #[cfg_attr(feature = "profile", profiling::function)]
    fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        if self
            .ui_thread
            .get()
            .is_some_and(|ui_thread| *ui_thread == thread::current().id())
        {
            return Err(anyhow!(
                "blocking database operation attempted on the GPUI thread"
            ));
        }
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

    /// Permanently identify the GPUI thread so a future call site cannot
    /// accidentally reintroduce a synchronous `recv()` into an event handler.
    pub(crate) fn register_ui_thread(&self) {
        let _ = self.ui_thread.set(thread::current().id());
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
             );
             CREATE TABLE IF NOT EXISTS collections (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS collection_folders (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
                 parent_id INTEGER REFERENCES collection_folders(id) ON DELETE CASCADE,
                 name TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_collection_folders_parent
                 ON collection_folders(collection_id, parent_id, position, id);
             CREATE TABLE IF NOT EXISTS saved_requests (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
                 folder_id INTEGER REFERENCES collection_folders(id) ON DELETE CASCADE,
                 name TEXT NOT NULL,
                 request_json TEXT NOT NULL,
                 params_json TEXT NOT NULL,
                 headers_json TEXT NOT NULL,
                 position INTEGER NOT NULL DEFAULT 0,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_saved_requests_parent
                 ON saved_requests(collection_id, folder_id, position, id);",
        )?;
        Self::migrate_add_request_auth(conn)?;
        Ok(())
    }

    /// Idempotently add the `request_auth` column. SQLite has no
    /// `ADD COLUMN IF NOT EXISTS`, so check `PRAGMA table_info` first. Old rows
    /// read back as NULL → `AuthConfig::default()`.
    fn migrate_add_request_auth(conn: &Connection) -> Result<()> {
        let mut stmt = conn.prepare("PRAGMA table_info(history)")?;
        let has_column = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "request_auth");
        drop(stmt);
        if !has_column {
            conn.execute("ALTER TABLE history ADD COLUMN request_auth TEXT", [])?;
        }
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
        auth: &AuthConfig,
    ) -> Result<i64> {
        let method = method.to_string();
        let url = url.to_string();
        let request_headers = request_headers.to_string();
        // Serialize body type + auth to JSON before crossing the channel.
        let body_json = serde_json::to_string(request_body).unwrap_or_default();
        // Defense in depth: callers should pass a history snapshot, but the DB
        // boundary also refuses to serialize drafts from inactive auth modes.
        let auth_json = serde_json::to_string(&auth.active_only()).unwrap_or_default();

        self.call(move |conn| {
            let timestamp = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO history (timestamp, method, url, request_headers, request_body, request_auth)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![timestamp, method, url, request_headers, body_json, auth_json],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Load recent history items (request only, no response - aligned with Postman)
    #[cfg_attr(feature = "profile", profiling::function)]
    pub fn load_recent_history(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, method, url, request_headers, request_body, request_auth
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
    #[cfg_attr(feature = "profile", profiling::function)]
    pub fn search_history(&self, query: &str, limit: usize) -> Result<Vec<HistoryItem>> {
        let pattern = format!("%{}%", escape_like(query));
        self.call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, timestamp, method, url, request_headers, request_body, request_auth
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
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))?;
            Ok(count as usize)
        })
    }

    // ===== Collections =====

    /// Load the complete collection/folder/request tree in display order.
    #[cfg_attr(feature = "profile", profiling::function)]
    pub fn load_collections(&self) -> Result<Vec<Collection>> {
        self.call(|conn| {
            let collection_rows = conn
                .prepare("SELECT id, name, position FROM collections ORDER BY position, id")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<Vec<(i64, String, i64)>>>()?;

            let folder_rows = conn
                .prepare(
                    "SELECT id, collection_id, parent_id, name, position
                     FROM collection_folders
                     ORDER BY collection_id, parent_id, position, id",
                )?
                .query_map([], |row| {
                    Ok(FolderRow {
                        id: row.get(0)?,
                        collection_id: row.get(1)?,
                        parent_id: row.get(2)?,
                        name: row.get(3)?,
                        position: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let request_rows = conn
                .prepare(
                    "SELECT id, collection_id, folder_id, name, request_json,
                            params_json, headers_json, position, created_at, updated_at
                     FROM saved_requests
                     ORDER BY collection_id, folder_id, position, id",
                )?
                .query_map([], |row| {
                    Ok(SavedRequestRow {
                        id: row.get(0)?,
                        collection_id: row.get(1)?,
                        folder_id: row.get(2)?,
                        name: row.get(3)?,
                        request_json: row.get(4)?,
                        params_json: row.get(5)?,
                        headers_json: row.get(6)?,
                        position: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            build_collection_tree(collection_rows, folder_rows, request_rows)
        })
    }

    /// Create an empty top-level collection.
    pub fn create_collection(&self, name: &str) -> Result<i64> {
        let name = require_name(name)?;
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO collections (name, position)
                 VALUES (?1, (SELECT COALESCE(MAX(position), -1) + 1 FROM collections))",
                params![name],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Create a folder under a collection or another folder.
    pub fn create_folder(
        &self,
        collection_id: i64,
        parent_id: Option<i64>,
        name: &str,
    ) -> Result<i64> {
        let name = require_name(name)?;
        self.call(move |conn| {
            let tx = conn.transaction()?;
            validate_folder_parent(&tx, collection_id, parent_id)?;
            let position: i64 = tx.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1
                 FROM collection_folders
                 WHERE collection_id = ?1 AND parent_id IS ?2",
                params![collection_id, parent_id],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO collection_folders (collection_id, parent_id, name, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![collection_id, parent_id, name, position],
            )?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            Ok(id)
        })
    }

    /// Insert a saved request and return its ID.
    pub fn insert_saved_request(
        &self,
        collection_id: i64,
        folder_id: Option<i64>,
        name: &str,
        request: &RequestData,
        params_state: &[ParamState],
        headers_state: &[HeaderState],
    ) -> Result<i64> {
        let name = require_name(name)?;
        let request = request.clone();
        let params_state = params_state.to_vec();
        let headers_state = headers_state.to_vec();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let id = insert_saved_request_tx(
                &tx,
                collection_id,
                folder_id,
                &name,
                &request,
                &params_state,
                &headers_state,
                None,
            )?;
            tx.commit()?;
            Ok(id)
        })
    }

    /// Update a saved request, including moving it to another folder.
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Retained for database callers and CRUD round-trip tests.
    pub fn update_saved_request(
        &self,
        id: i64,
        collection_id: i64,
        folder_id: Option<i64>,
        name: &str,
        request: &RequestData,
        params_state: &[ParamState],
        headers_state: &[HeaderState],
    ) -> Result<()> {
        let name = require_name(name)?;
        let request = request.clone();
        let params_state = params_state.to_vec();
        let headers_state = headers_state.to_vec();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            validate_folder_parent(&tx, collection_id, folder_id)?;
            let (request_json, params_json, headers_json) =
                encode_saved_request(&request, &params_state, &headers_state)?;
            let now = chrono::Utc::now().to_rfc3339();
            let changed = tx.execute(
                "UPDATE saved_requests
                 SET collection_id = ?1, folder_id = ?2, name = ?3,
                     request_json = ?4, params_json = ?5, headers_json = ?6,
                     updated_at = ?7
                 WHERE id = ?8",
                params![
                    collection_id,
                    folder_id,
                    name,
                    request_json,
                    params_json,
                    headers_json,
                    now,
                    id,
                ],
            )?;
            if changed == 0 {
                return Err(anyhow!("saved request {} does not exist", id));
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn load_saved_request(&self, id: i64) -> Result<Option<SavedRequest>> {
        self.call(move |conn| {
            let raw = conn
                .query_row(
                    "SELECT id, collection_id, folder_id, name, request_json,
                            params_json, headers_json, position, created_at, updated_at
                     FROM saved_requests WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(SavedRequestRow {
                            id: row.get(0)?,
                            collection_id: row.get(1)?,
                            folder_id: row.get(2)?,
                            name: row.get(3)?,
                            request_json: row.get(4)?,
                            params_json: row.get(5)?,
                            headers_json: row.get(6)?,
                            position: row.get(7)?,
                            created_at: row.get(8)?,
                            updated_at: row.get(9)?,
                        })
                    },
                )
                .optional()?;
            raw.map(decode_saved_request).transpose()
        })
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        let name = require_name(name)?;
        self.call(move |conn| {
            let changed = conn.execute(
                "UPDATE collections SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
            if changed == 0 {
                return Err(anyhow!("collection {} does not exist", id));
            }
            Ok(())
        })
    }

    pub fn rename_folder(&self, id: i64, name: &str) -> Result<()> {
        let name = require_name(name)?;
        self.call(move |conn| {
            let changed = conn.execute(
                "UPDATE collection_folders SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
            if changed == 0 {
                return Err(anyhow!("folder {} does not exist", id));
            }
            Ok(())
        })
    }

    pub fn rename_saved_request(&self, id: i64, name: &str) -> Result<()> {
        let name = require_name(name)?;
        self.call(move |conn| {
            let changed = conn.execute(
                "UPDATE saved_requests SET name = ?1, updated_at = ?2 WHERE id = ?3",
                params![name, chrono::Utc::now().to_rfc3339(), id],
            )?;
            if changed == 0 {
                return Err(anyhow!("saved request {} does not exist", id));
            }
            Ok(())
        })
    }

    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            conn.execute("DELETE FROM collections WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    pub fn delete_folder(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            conn.execute("DELETE FROM collection_folders WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    pub fn delete_saved_request(&self, id: i64) -> Result<()> {
        self.call(move |conn| {
            conn.execute("DELETE FROM saved_requests WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// Import or copy a complete collection tree in one transaction.
    pub fn insert_collection_tree(&self, collection: &Collection, name: &str) -> Result<i64> {
        let name = require_name(name)?;
        let collection = collection.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let id = insert_collection_tree_tx(&tx, &collection, &name)?;
            tx.commit()?;
            Ok(id)
        })
    }

    pub fn duplicate_collection(&self, id: i64) -> Result<i64> {
        let collections = self.load_collections()?;
        let collection = collections
            .iter()
            .find(|collection| collection.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("collection {} does not exist", id))?;
        let name = copy_name(&collection.name);
        self.insert_collection_tree(&collection, &name)
    }

    pub fn duplicate_folder(&self, id: i64) -> Result<i64> {
        let collections = self.load_collections()?;
        let folder = collections
            .iter()
            .find_map(|collection| find_folder(&collection.folders, id))
            .cloned()
            .ok_or_else(|| anyhow!("folder {} does not exist", id))?;
        let collection_id = folder.collection_id;
        let parent_id = folder.parent_id;
        let name = copy_name(&folder.name);
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut copy = folder.clone();
            copy.name = name;
            let new_id = insert_folder_tree_tx(&tx, collection_id, parent_id, &copy)?;
            tx.commit()?;
            Ok(new_id)
        })
    }

    pub fn duplicate_saved_request(&self, id: i64) -> Result<i64> {
        let request = self
            .load_saved_request(id)?
            .ok_or_else(|| anyhow!("saved request {} does not exist", id))?;
        let name = copy_name(&request.name);
        self.insert_saved_request(
            request.collection_id,
            request.folder_id,
            &name,
            &request.request,
            &request.params_state,
            &request.headers_state,
        )
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
                result.push(Environment {
                    id,
                    name,
                    variables,
                });
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

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rename_environment(&self, id: i64, name: &str) -> Result<()> {
        let name = name.to_string();
        self.call(move |conn| {
            conn.execute(
                "UPDATE environments SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;
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
    #[cfg_attr(not(test), allow(dead_code))]
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
                    params![
                        environment_id,
                        v.enabled as i64,
                        v.key,
                        v.value,
                        position as i64
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Persist one editor snapshot in a transaction, but only if it is still
    /// current when the serialized database thread reaches this job.
    pub fn save_environment_if_current(
        &self,
        environment_id: i64,
        name: &str,
        vars: &[EnvVar],
        epoch: Arc<AtomicU64>,
        generation: u64,
    ) -> Result<()> {
        let name = name.to_string();
        let vars = vars.to_vec();
        self.call(move |conn| {
            if epoch.load(Ordering::Acquire) != generation {
                return Ok(());
            }
            let tx = conn.transaction()?;
            if !name.trim().is_empty() {
                tx.execute(
                    "UPDATE environments SET name = ?1 WHERE id = ?2",
                    params![name, environment_id],
                )?;
            }
            tx.execute(
                "DELETE FROM env_variables WHERE environment_id = ?1",
                params![environment_id],
            )?;
            for (position, variable) in vars.iter().enumerate() {
                tx.execute(
                    "INSERT INTO env_variables (environment_id, enabled, key, value, position)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        environment_id,
                        variable.enabled as i64,
                        variable.key,
                        variable.value,
                        position as i64
                    ],
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
                    conn.execute(
                        "DELETE FROM app_meta WHERE key = 'active_environment_id'",
                        [],
                    )?;
                }
            }
            Ok(())
        })
    }

    /// Load the app-wide HTTP safeguards. Missing or malformed values are
    /// intentionally non-fatal: an upgrade should always start with safe
    /// defaults rather than leave the application unusable.
    pub fn load_app_settings(&self) -> Result<AppSettings> {
        self.call(|conn| {
            let value: Option<String> = conn
                .query_row(
                    "SELECT value FROM app_meta WHERE key = 'app_settings'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(value
                .as_deref()
                .and_then(|json| serde_json::from_str::<AppSettings>(json).ok())
                .unwrap_or_default()
                .normalized())
        })
    }

    /// Persist all General settings atomically in the existing app metadata
    /// table. Keeping them in one JSON value makes future settings additions
    /// backwards-compatible and avoids a schema migration for every field.
    pub fn save_app_settings(&self, settings: &AppSettings) -> Result<()> {
        let json = serde_json::to_string(&settings.clone().normalized())?;
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO app_meta (key, value) VALUES ('app_settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![json],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::{AppSettings, AuthConfig, AuthType};

    fn mem_db() -> Database {
        Database::new_in_memory()
    }

    #[test]
    fn app_settings_round_trip_through_metadata() {
        let db = mem_db();
        let settings = AppSettings {
            connect_timeout_ms: 1_500,
            read_timeout_ms: 2_500,
            total_timeout_ms: 3_500,
            max_response_size_bytes: 12 * 1024 * 1024,
        };
        db.save_app_settings(&settings).unwrap();
        assert_eq!(db.load_app_settings().unwrap(), settings);
    }

    #[test]
    fn registered_ui_thread_cannot_wait_for_database() {
        let db = mem_db();
        db.register_ui_thread();

        let error = db
            .load_recent_history(1)
            .expect_err("UI-thread database calls must fail before recv");
        assert!(
            error
                .to_string()
                .contains("blocking database operation attempted on the GPUI thread")
        );
    }

    #[test]
    fn stale_environment_snapshot_is_not_persisted() {
        let db = mem_db();
        let id = db.create_environment("original").unwrap();
        let epoch = Arc::new(AtomicU64::new(2));

        db.save_environment_if_current(
            id,
            "stale",
            &[EnvVar {
                enabled: true,
                key: "token".into(),
                value: "old".into(),
            }],
            epoch.clone(),
            1,
        )
        .unwrap();
        let environment = db.load_environments().unwrap().remove(0);
        assert_eq!(environment.name, "original");
        assert!(environment.variables.is_empty());

        db.save_environment_if_current(
            id,
            "current",
            &[EnvVar {
                enabled: true,
                key: "token".into(),
                value: "new".into(),
            }],
            epoch,
            2,
        )
        .unwrap();
        let environment = db.load_environments().unwrap().remove(0);
        assert_eq!(environment.name, "current");
        assert_eq!(environment.variables[0].value, "new");
    }

    #[test]
    fn environment_save_epochs_are_independent() {
        let db = mem_db();
        let first_id = db.create_environment("first").unwrap();
        let second_id = db.create_environment("second").unwrap();
        let first_epoch = Arc::new(AtomicU64::new(1));
        let second_epoch = Arc::new(AtomicU64::new(1));

        db.save_environment_if_current(first_id, "first edited", &[], first_epoch, 1)
            .unwrap();
        db.save_environment_if_current(second_id, "second edited", &[], second_epoch, 1)
            .unwrap();

        let environments = db.load_environments().unwrap();
        assert_eq!(
            environments
                .iter()
                .find(|environment| environment.id == first_id)
                .unwrap()
                .name,
            "first edited"
        );
        assert_eq!(
            environments
                .iter()
                .find(|environment| environment.id == second_id)
                .unwrap()
                .name,
            "second edited"
        );
    }

    #[test]
    fn migration_adds_request_auth_and_old_rows_default() {
        // Simulate a pre-feature database: history table WITHOUT request_auth.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp TEXT NOT NULL, method TEXT NOT NULL, url TEXT NOT NULL,
                 request_headers TEXT, request_body TEXT,
                 status_code INTEGER, duration_ms INTEGER,
                 response_headers TEXT, response_body TEXT
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO history (timestamp, method, url, request_headers, request_body)
             VALUES ('t','GET','https://x','[]','null')",
            [],
        )
        .unwrap();

        // Migration is idempotent and adds the column.
        Database::migrate_add_request_auth(&conn).unwrap();
        Database::migrate_add_request_auth(&conn).unwrap(); // second run is a no-op

        let db = Database::spawn(conn);
        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].request.auth.auth_type, AuthType::None);
    }

    #[test]
    fn history_roundtrips_auth() {
        let db = mem_db();
        let auth = AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "abc".into(),
            ..Default::default()
        };
        db.insert_history("GET", "https://x", "[]", &BodyType::None, &auth)
            .unwrap();
        let items = db.load_recent_history(10).unwrap();
        assert_eq!(items[0].request.auth.auth_type, AuthType::Bearer);
        assert_eq!(items[0].request.auth.bearer_token, "abc");
    }

    #[test]
    fn history_storage_drops_inactive_auth_fields() {
        let db = mem_db();
        let auth = AuthConfig {
            auth_type: AuthType::None,
            bearer_token: "must-not-persist".into(),
            basic_username: "must-not-persist".into(),
            basic_password: "must-not-persist".into(),
            api_key_name: "must-not-persist".into(),
            api_key_value: "must-not-persist".into(),
        };

        db.insert_history(
            "GET",
            "{{base_url}}/private",
            r#"[["Authorization","Bearer {{token}}"]]"#,
            &BodyType::None,
            &auth,
        )
        .unwrap();

        let item = db.load_recent_history(1).unwrap().remove(0);
        assert_eq!(item.request.url, "{{base_url}}/private");
        assert_eq!(
            item.request.headers,
            vec![("Authorization".into(), "Bearer {{token}}".into())]
        );
        assert_eq!(item.request.auth, AuthConfig::default());
    }

    #[test]
    fn crud_and_active() {
        let db = mem_db();
        let id = db.create_environment("dev").unwrap();
        db.replace_variables(
            id,
            &[
                EnvVar {
                    enabled: true,
                    key: "baseUrl".into(),
                    value: "http://x".into(),
                },
                EnvVar {
                    enabled: false,
                    key: "token".into(),
                    value: "abc".into(),
                },
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
        db.insert_history(
            "GET",
            "https://api.test/x",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
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
        db.insert_history(
            "GET",
            "https://api.test/users",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
        .unwrap();
        db.insert_history(
            "POST",
            "https://api.test/login",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
        .unwrap();
        db.insert_history(
            "DELETE",
            "https://api.test/orders/1",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
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
        db.insert_history(
            "GET",
            "https://api.test/a%b",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
        .unwrap();
        db.insert_history(
            "GET",
            "https://api.test/a_b",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
        .unwrap();
        db.insert_history(
            "GET",
            "https://api.test/axb",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
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
        db.insert_history(
            "GET",
            "https://api.test/users",
            "[]",
            &crate::types::BodyType::None,
            &crate::types::AuthConfig::default(),
        )
        .unwrap();
        let r = db.search_history("", 10).unwrap();
        assert_eq!(r.len(), 1);
    }

    fn saved_request_fixture(
        name: &str,
        url: &str,
    ) -> (RequestData, Vec<ParamState>, Vec<HeaderState>) {
        let request = RequestData {
            method: HttpMethod::POST,
            url: url.to_string(),
            headers: vec![("X-Enabled".into(), "yes".into())],
            body: BodyType::Raw {
                content: "{\"ok\":true}".into(),
                subtype: crate::types::RawSubtype::Json,
            },
            auth: AuthConfig {
                auth_type: AuthType::Bearer,
                bearer_token: "token".into(),
                ..Default::default()
            },
        };
        let params = vec![
            ParamState {
                enabled: true,
                key: "page".into(),
                value: "1".into(),
            },
            ParamState {
                enabled: false,
                key: "debug".into(),
                value: "true".into(),
            },
        ];
        let headers = vec![
            HeaderState {
                enabled: true,
                key: "X-Enabled".into(),
                value: "yes".into(),
                header_type: crate::types::HeaderType::Custom,
                predefined: None,
            },
            HeaderState {
                enabled: false,
                key: "X-Disabled".into(),
                value: "no".into(),
                header_type: crate::types::HeaderType::Custom,
                predefined: None,
            },
        ];
        let _ = name;
        (request, params, headers)
    }

    #[test]
    fn collections_roundtrip_nested_tree_and_editor_state() {
        let db = mem_db();
        let collection_id = db.create_collection("Demo").unwrap();
        let root_folder_id = db.create_folder(collection_id, None, "Root").unwrap();
        let child_folder_id = db
            .create_folder(collection_id, Some(root_folder_id), "Child")
            .unwrap();
        let (request, params, headers) =
            saved_request_fixture("Nested", "https://api.test/items?page=1");
        let root_request_id = db
            .insert_saved_request(
                collection_id,
                None,
                "Root request",
                &request,
                &params,
                &headers,
            )
            .unwrap();
        let nested_request_id = db
            .insert_saved_request(
                collection_id,
                Some(child_folder_id),
                "Nested",
                &request,
                &params,
                &headers,
            )
            .unwrap();

        let collections = db.load_collections().unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].requests[0].id, root_request_id);
        assert_eq!(collections[0].folders[0].name, "Root");
        assert_eq!(collections[0].folders[0].folders[0].name, "Child");
        let saved = &collections[0].folders[0].folders[0].requests[0];
        assert_eq!(saved.id, nested_request_id);
        assert_eq!(saved.request, request);
        assert_eq!(saved.params_state, params);
        assert_eq!(saved.headers_state, headers);
    }

    #[test]
    fn collection_update_duplicate_and_cascade_delete() {
        let db = mem_db();
        let collection_id = db.create_collection("Demo").unwrap();
        let folder_id = db.create_folder(collection_id, None, "Folder").unwrap();
        let (request, params, headers) = saved_request_fixture("Request", "https://api.test/a");
        let request_id = db
            .insert_saved_request(
                collection_id,
                Some(folder_id),
                "Request",
                &request,
                &params,
                &headers,
            )
            .unwrap();

        let mut changed = request.clone();
        changed.url = "https://api.test/b".into();
        db.update_saved_request(
            request_id,
            collection_id,
            Some(folder_id),
            "Updated",
            &changed,
            &[],
            &[],
        )
        .unwrap();
        let updated = db.load_saved_request(request_id).unwrap().unwrap();
        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.request.url, "https://api.test/b");
        assert!(updated.params_state.is_empty());

        let copied_request_id = db.duplicate_saved_request(request_id).unwrap();
        assert_eq!(
            db.load_saved_request(copied_request_id)
                .unwrap()
                .unwrap()
                .name,
            "Updated (Copy)"
        );
        let copied_folder_id = db.duplicate_folder(folder_id).unwrap();
        let copied_tree = db.load_collections().unwrap();
        assert!(
            copied_tree[0]
                .folders
                .iter()
                .any(|folder| folder.id == copied_folder_id)
        );

        // Folder deletion cascades to both the original and its copied request;
        // the unrelated root collection remains intact.
        db.delete_folder(folder_id).unwrap();
        assert!(db.load_saved_request(request_id).unwrap().is_none());
        assert!(
            db.load_collections().unwrap()[0]
                .folders
                .iter()
                .all(|folder| folder.id != folder_id)
        );
        db.delete_collection(collection_id).unwrap();
        assert!(db.load_collections().unwrap().is_empty());
    }

    #[test]
    fn inserting_a_collection_tree_rebuilds_real_parent_ids_transactionally() {
        let db = mem_db();
        let (request, params, headers) =
            saved_request_fixture("Imported", "https://api.test/imported");
        let imported = Collection {
            id: 0,
            name: "Imported".into(),
            position: 0,
            requests: vec![],
            folders: vec![CollectionFolder {
                id: 999,
                collection_id: 0,
                parent_id: None,
                name: "Outer".into(),
                position: 0,
                requests: vec![],
                folders: vec![CollectionFolder {
                    id: 1000,
                    collection_id: 0,
                    parent_id: Some(999),
                    name: "Inner".into(),
                    position: 0,
                    requests: vec![SavedRequest {
                        id: 0,
                        collection_id: 0,
                        folder_id: Some(1000),
                        name: "Imported".into(),
                        request,
                        params_state: params,
                        headers_state: headers,
                        position: 0,
                        created_at: String::new(),
                        updated_at: String::new(),
                    }],
                    folders: vec![],
                }],
            }],
        };
        let collection_id = db.insert_collection_tree(&imported, "Imported").unwrap();
        let tree = db.load_collections().unwrap();
        assert_eq!(tree[0].id, collection_id);
        let outer = &tree[0].folders[0];
        let inner = &outer.folders[0];
        assert_eq!(outer.collection_id, collection_id);
        assert_eq!(inner.parent_id, Some(outer.id));
        assert_eq!(inner.requests[0].collection_id, collection_id);
        assert_eq!(inner.requests[0].folder_id, Some(inner.id));
    }
}
