//! Platform-native database paths and recoverable, non-destructive legacy adoption.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

// Keep this identity stable across releases and aligned with the bundle identifier.
const APP_ID: &str = "com.poopman.app";

pub(crate) struct DatabasePaths {
    legacy: PathBuf,
    destination: PathBuf,
}

impl DatabasePaths {
    pub(crate) fn resolve() -> Result<Self> {
        let data = dirs::data_local_dir().context("Cannot find application data directory")?;
        let home = dirs::home_dir().context("Cannot find home directory")?;
        Ok(Self::from_roots(&data, &home))
    }

    /// Inject roots without changing process-wide environment variables in tests.
    pub(crate) fn from_roots(data: &Path, home: &Path) -> Self {
        Self {
            legacy: home.join(".poopman").join("history.db"),
            destination: data.join(APP_ID).join("history.db"),
        }
    }

    pub(crate) fn prepare(&self) -> Result<PathBuf> {
        // The destination always wins, even if the legacy database is newer.
        // Never silently fall back to old data if opening the destination fails.
        if database_exists(&self.destination)? {
            return Ok(self.destination.clone());
        }
        let parent = self
            .destination
            .parent()
            .context("Missing database directory")?;
        fs::create_dir_all(parent).context("Cannot create application data directory")?;
        if database_exists(&self.legacy)? {
            let snapshot = self
                .snapshot(parent)
                .context("Cannot back up legacy database")?;
            publish(snapshot, &self.destination).context("Cannot install migrated database")?;
        }
        Ok(self.destination.clone())
    }

    fn snapshot(&self, parent: &Path) -> Result<NamedTempFile> {
        let snapshot = tempfile::Builder::new()
            .prefix(".history-migration-")
            .suffix(".db")
            .tempfile_in(parent)?;
        // Do not copy the database file directly: committed data may still be in
        // its WAL. No CREATE flag: a disappearing source must not become an empty DB.
        // Read/write access lets SQLite recover a hot rollback journal if necessary.
        let source = Connection::open_with_flags(&self.legacy, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        source.backup(rusqlite::MAIN_DB, snapshot.path(), None)?;
        // backup() closes the destination connection before we sync or publish it.
        snapshot.as_file().sync_all()?;
        Ok(snapshot)
    }
}

fn database_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            // Also reject dangling symlinks and directories instead of treating
            // them as absent and potentially overwriting an existing entry.
            if !fs::metadata(path)?.is_file() {
                bail!("Database path is not a file: {}", path.display());
            }
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn publish(snapshot: NamedTempFile, destination: &Path) -> Result<()> {
    // Same-directory publication prevents cross-filesystem moves. The destination
    // name appears only after the complete snapshot is closed and flushed, and
    // persist_noclobber cannot overwrite a database created by another launch.
    match snapshot.persist_noclobber(destination) {
        Ok(_) => {}
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            database_exists(destination)?;
        }
        Err(error) => return Err(error.error.into()),
    }
    // On Unix, persist the directory entry as well as the file contents. If the
    // process dies earlier, the untouched legacy DB allows the next launch to retry.
    #[cfg(unix)]
    fs::File::open(destination.parent().context("Missing database directory")?)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> DatabasePaths {
        DatabasePaths::from_roots(&root.join("native"), &root.join("home"))
    }

    fn fixture(path: &Path, value: &str) -> Connection {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE payload (value TEXT NOT NULL);")
            .unwrap();
        conn.execute("INSERT INTO payload VALUES (?1)", [value])
            .unwrap();
        conn
    }

    fn contents(path: &Path) -> String {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap()
            .query_row("SELECT value FROM payload", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn injected_roots_use_stable_identity() {
        let paths = DatabasePaths::from_roots(Path::new("data"), Path::new("home"));
        assert_eq!(
            paths.destination,
            Path::new("data/com.poopman.app/history.db")
        );
        assert_eq!(paths.legacy, Path::new("home/.poopman/history.db"));
    }

    #[test]
    fn fresh_install_creates_only_native_directory() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        assert_eq!(paths.prepare().unwrap(), paths.destination);
        assert!(paths.destination.parent().unwrap().is_dir());
        assert!(!paths.destination.exists());
        assert!(!paths.legacy.parent().unwrap().exists());
    }

    #[test]
    fn legacy_only_is_backed_up_and_retained() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.legacy, "legacy"));
        let original = fs::read(&paths.legacy).unwrap();
        paths.prepare().unwrap();
        assert_eq!(contents(&paths.destination), "legacy");
        assert_eq!(fs::read(&paths.legacy).unwrap(), original);
        assert_eq!(
            fs::read_dir(paths.destination.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn destination_only_is_used_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.destination, "native"));
        let original = fs::read(&paths.destination).unwrap();
        paths.prepare().unwrap();
        assert_eq!(fs::read(&paths.destination).unwrap(), original);
        assert!(!paths.legacy.exists());
    }

    #[test]
    fn both_exist_destination_wins_even_when_legacy_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.destination, "native"));
        fs::create_dir_all(paths.legacy.parent().unwrap()).unwrap();
        fs::write(&paths.legacy, b"broken legacy database").unwrap();
        paths.prepare().unwrap();
        assert_eq!(contents(&paths.destination), "native");
        assert_eq!(fs::read(&paths.legacy).unwrap(), b"broken legacy database");
    }

    #[test]
    fn committed_wal_data_is_included() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let source = fixture(&paths.legacy, "before WAL");
        source
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
            UPDATE payload SET value = 'committed in WAL';",
            )
            .unwrap();
        // Keep the source open so closing it cannot checkpoint the WAL for us.
        assert!(paths.legacy.with_extension("db-wal").exists());
        paths.prepare().unwrap();
        assert_eq!(contents(&paths.destination), "committed in WAL");
        assert_eq!(contents(&paths.legacy), "committed in WAL");
    }

    #[test]
    fn interrupted_snapshot_is_ignored_and_migration_retries() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.legacy, "complete"));
        fs::create_dir_all(paths.destination.parent().unwrap()).unwrap();
        let abandoned = paths
            .destination
            .with_file_name(".history-migration-interrupted.db");
        fs::write(&abandoned, b"incomplete snapshot").unwrap();
        paths.prepare().unwrap();
        assert_eq!(contents(&paths.destination), "complete");
        // Never delete another launch's temporary file.
        assert_eq!(fs::read(abandoned).unwrap(), b"incomplete snapshot");
    }

    #[test]
    fn interrupted_after_publication_uses_destination_on_restart() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let source = fixture(&paths.legacy, "snapshot");
        paths.prepare().unwrap();
        source
            .execute("UPDATE payload SET value = 'later legacy edit'", [])
            .unwrap();
        paths.prepare().unwrap();
        assert_eq!(contents(&paths.destination), "snapshot");
        assert_eq!(contents(&paths.legacy), "later legacy edit");
    }

    #[test]
    fn racing_destination_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.legacy, "legacy"));
        fs::create_dir_all(paths.destination.parent().unwrap()).unwrap();
        let snapshot = paths.snapshot(paths.destination.parent().unwrap()).unwrap();
        drop(fixture(&paths.destination, "other launch"));
        publish(snapshot, &paths.destination).unwrap();
        assert_eq!(contents(&paths.destination), "other launch");
    }

    #[test]
    fn failed_backup_keeps_legacy_and_does_not_publish_partial_database() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        fs::create_dir_all(paths.legacy.parent().unwrap()).unwrap();
        fs::write(&paths.legacy, b"invalid database").unwrap();
        assert!(paths.prepare().is_err());
        assert_eq!(fs::read(&paths.legacy).unwrap(), b"invalid database");
        assert!(!paths.destination.exists());
        assert_eq!(
            fs::read_dir(paths.destination.parent().unwrap())
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn invalid_destination_does_not_fall_back_to_legacy() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        drop(fixture(&paths.legacy, "legacy"));
        fs::create_dir_all(&paths.destination).unwrap();
        assert!(paths.prepare().is_err());
        assert_eq!(contents(&paths.legacy), "legacy");
    }
}
