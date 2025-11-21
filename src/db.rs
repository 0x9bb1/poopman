use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::types::{HistoryItem, HttpMethod, RequestData, ResponseData};

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

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Get the database file path
    fn get_db_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        Ok(home.join(".poopman").join("history.db"))
    }

    /// Insert a new history item
    pub fn insert_history(
        &self,
        method: &str,
        url: &str,
        request_headers: &str,
        request_body: &crate::types::BodyType,
        status_code: Option<u16>,
        duration_ms: Option<u64>,
        response_headers: Option<&str>,
        response_body: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();

        let timestamp = chrono::Utc::now().to_rfc3339();

        // Serialize body type to JSON
        let body_json = serde_json::to_string(request_body).unwrap_or_default();

        conn.execute(
            "INSERT INTO history (timestamp, method, url, request_headers, request_body, status_code, duration_ms, response_headers, response_body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                timestamp,
                method,
                url,
                request_headers,
                body_json,
                status_code.map(|s| s as i64),
                duration_ms.map(|d| d as i64),
                response_headers,
                response_body,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Load recent history items
    pub fn load_recent_history(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, url, request_headers, request_body,
                    status_code, duration_ms, response_headers, response_body
             FROM history
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;

        let items = stmt.query_map([limit], |row| {
            let id: i64 = row.get(0)?;
            let timestamp: String = row.get(1)?;
            let method: String = row.get(2)?;
            let url: String = row.get(3)?;
            let request_headers: String = row.get(4)?;
            let request_body: String = row.get(5)?;
            let status_code: Option<i64> = row.get(6)?;
            let duration_ms: Option<i64> = row.get(7)?;
            let response_headers: Option<String> = row.get(8)?;
            let response_body: Option<String> = row.get(9)?;

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

            let response = if let (Some(status), Some(duration)) = (status_code, duration_ms) {
                let resp_headers: Vec<(String, String)> = response_headers
                    .and_then(|h| serde_json::from_str(&h).ok())
                    .unwrap_or_default();

                Some(ResponseData {
                    status: Some(status as u16),
                    duration_ms: duration as u64,
                    headers: resp_headers,
                    body: response_body.unwrap_or_default(),
                })
            } else {
                None
            };

            Ok(HistoryItem::new(id, timestamp, request, response))
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
}
