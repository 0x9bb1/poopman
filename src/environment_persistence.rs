//! Ordered environment writes. Commands are submitted synchronously by the UI;
//! only their completion is asynchronous. The worker owns debounced snapshots,
//! so they survive dropped views/futures and can be flushed before quitting.

use anyhow::{Result, anyhow};
use futures::channel::oneshot;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use crate::db::Database;

type Job = Box<dyn FnOnce(&Database) -> Option<String> + Send>;
enum Command {
    Edit(i64, Job),
    Run(Job),
    Flush(oneshot::Sender<Result<()>>),
}

#[derive(Clone)]
pub(crate) struct EnvironmentPersistence {
    tx: mpsc::Sender<Command>,
    closing: Arc<AtomicBool>,
}

impl EnvironmentPersistence {
    pub(crate) fn new(db: Arc<Database>) -> Self {
        Self::with_debounce(db, Duration::from_millis(180))
    }

    fn with_debounce(db: Arc<Database>, debounce: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut pending: Vec<(i64, Job)> = Vec::new();
            let mut deadline = Instant::now();
            let mut last_error = None;
            loop {
                let command = if pending.is_empty() {
                    rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
                } else {
                    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                };
                match command {
                    Ok(Command::Edit(id, job)) => {
                        // Move replacements to the end to preserve the order of
                        // the latest accepted snapshots across environments.
                        pending.retain(|(pending_id, _)| *pending_id != id);
                        pending.push((id, job));
                        deadline = Instant::now() + debounce;
                    }
                    command => {
                        for (_, job) in pending.drain(..) {
                            record_error(&mut last_error, job(&db));
                        }
                        match command {
                            Ok(Command::Run(job)) => record_error(&mut last_error, job(&db)),
                            Ok(Command::Flush(reply)) => {
                                let result = last_error.take().map_or(Ok(()), |e| Err(anyhow!(e)));
                                let _ = reply.send(result);
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                            Err(RecvTimeoutError::Timeout) => {}
                            Ok(Command::Edit(..)) => unreachable!(),
                        }
                    }
                }
            }
        });
        Self {
            tx,
            closing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// This must remain a regular function: enqueue before returning the future,
    /// never when a detached task happens to poll it.
    pub(crate) fn submit<T: Send + 'static, F: FnOnce(&Database) -> Result<T> + Send + 'static>(
        &self,
        edit_id: Option<i64>,
        operation: F,
    ) -> impl Future<Output = Result<T>> + use<T, F> {
        let (reply, result) = oneshot::channel();
        if self.is_closing() {
            let _ = reply.send(Err(anyhow!("environment persistence is shutting down")));
        } else {
            let job: Job = Box::new(move |db| {
                let result = operation(db);
                let error = result.as_ref().err().map(ToString::to_string);
                let _ = reply.send(result);
                error
            });
            let command = match edit_id {
                Some(id) => Command::Edit(id, job),
                None => Command::Run(job),
            };
            let _ = self.tx.send(command);
        }
        async move {
            result
                .await
                .map_err(|_| anyhow!("environment save was superseded or worker stopped"))?
        }
    }

    pub(crate) fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }

    /// Freeze accepted mutations and force all queued snapshots to disk. The
    /// caller bounds the wait; no UI callback is needed to complete this future.
    pub(crate) fn shutdown(&self) -> impl Future<Output = Result<()>> + use<> {
        self.closing.store(true, Ordering::Release);
        let (reply, result) = oneshot::channel();
        let _ = self.tx.send(Command::Flush(reply));
        async move {
            result
                .await
                .map_err(|_| anyhow!("environment persistence worker stopped"))?
        }
    }
}

fn record_error(last_error: &mut Option<String>, error: Option<String>) {
    if let Some(error) = error {
        log::error!("Environment persistence failed: {error}");
        *last_error = Some(error);
    }
}

pub(crate) async fn finish_shutdown(
    flush: impl Future<Output = Result<()>>,
    timeout: impl Future<Output = ()>,
) -> Result<()> {
    match futures::future::select(Box::pin(flush), Box::pin(timeout)).await {
        futures::future::Either::Left((result, _)) => result,
        futures::future::Either::Right(_) => {
            Err(anyhow!("timed out saving environments before quitting"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EnvVar;
    use std::sync::atomic::AtomicU64;

    fn wait<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(futures::executor::block_on(future));
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("persistence must complete without waiting for debounce")
    }

    fn edit(
        persistence: &EnvironmentPersistence,
        id: i64,
        value: &str,
    ) -> impl Future<Output = Result<()>> + use<> {
        let value = value.to_string();
        persistence.submit(Some(id), move |db| {
            db.save_environment_if_current(
                id,
                "Edited",
                &[EnvVar {
                    enabled: true,
                    key: "token".into(),
                    value,
                }],
                Arc::new(AtomicU64::new(1)),
                1,
            )
        })
    }

    fn queue(db: Arc<Database>) -> EnvironmentPersistence {
        // No timing race: only an explicit barrier can expire these edits.
        EnvironmentPersistence::with_debounce(db, Duration::from_secs(3600))
    }

    #[test]
    fn shutdown_flushes_latest_snapshots_for_multiple_environments_before_debounce() {
        let db = Arc::new(Database::new_in_memory());
        let a = db.create_environment("A").unwrap();
        let b = db.create_environment("B").unwrap();
        let persistence = queue(db.clone());
        let stale = edit(&persistence, a, "old");
        let b_save = edit(&persistence, b, "B value");
        let a_save = edit(&persistence, a, "latest");
        wait(persistence.shutdown()).unwrap();
        assert!(wait(stale).is_err());
        wait(a_save).unwrap();
        wait(b_save).unwrap();
        let environments = db.load_environments().unwrap();
        assert_eq!(environments[0].variables[0].value, "latest");
        assert_eq!(environments[1].variables[0].value, "B value");
    }

    #[test]
    fn rapid_switches_are_enqueued_before_completion_futures_are_polled() {
        let db = Arc::new(Database::new_in_memory());
        let a = db.create_environment("A").unwrap();
        let b = db.create_environment("B").unwrap();
        let persistence = queue(db.clone());
        let (release, gate) = mpsc::channel();
        let blocked = persistence.submit(None, move |_| {
            gate.recv()?;
            Ok(())
        });
        // The worker is stalled while all UI operations are accepted. Dropping
        // their completion futures must neither cancel nor reorder the writes.
        for id in [Some(a), Some(b), None, Some(a), Some(b)] {
            drop(persistence.submit(None, move |db| db.set_active_environment_id(id)));
        }
        let shutdown = persistence.shutdown();
        release.send(()).unwrap();
        wait(shutdown).unwrap();
        wait(blocked).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), Some(b));
    }

    #[test]
    fn edits_are_flushed_before_create_delete_and_active_changes() {
        let db = Arc::new(Database::new_in_memory());
        let a = db.create_environment("A").unwrap();
        db.set_active_environment_id(Some(a)).unwrap();
        let persistence = queue(db.clone());
        drop(edit(&persistence, a, "pending"));
        let create = persistence.submit(None, move |db| {
            assert_eq!(db.load_environments()?[0].variables[0].value, "pending");
            db.create_environment("B")
        });
        let b = wait(create).unwrap();
        drop(persistence.submit(None, move |db| db.set_active_environment_id(Some(b))));
        drop(persistence.submit(None, move |db| db.delete_environment(a)));
        wait(persistence.shutdown()).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), Some(b));
        assert_eq!(db.load_environments().unwrap().len(), 1);
    }

    #[test]
    fn deleting_the_persisted_active_environment_clears_it() {
        let db = Arc::new(Database::new_in_memory());
        let a = db.create_environment("A").unwrap();
        let persistence = queue(db.clone());
        drop(persistence.submit(None, move |db| db.set_active_environment_id(Some(a))));
        drop(persistence.submit(None, move |db| db.delete_environment(a)));
        wait(persistence.shutdown()).unwrap();
        assert_eq!(db.get_active_environment_id().unwrap(), None);
    }

    #[test]
    fn failure_is_reported_at_shutdown_without_stalling_later_writes() {
        let db = Arc::new(Database::new_in_memory());
        let persistence = queue(db.clone());
        drop(persistence.submit::<(), _>(None, |_| Err(anyhow!("simulated disk failure"))));
        drop(persistence.submit(None, |db| db.create_environment("Still processed")));
        let error = wait(persistence.shutdown()).unwrap_err();
        assert!(error.to_string().contains("simulated disk failure"));
        assert_eq!(db.load_environments().unwrap().len(), 1);
        assert!(wait(persistence.submit(None, |db| db.create_environment("Too late"))).is_err());
        assert_eq!(db.load_environments().unwrap().len(), 1);
    }

    #[test]
    fn shutdown_timeout_does_not_wait_for_a_stuck_worker() {
        let result = wait(finish_shutdown(
            futures::future::pending(),
            futures::future::ready(()),
        ));
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[test]
    fn expired_debounce_saves_without_shutdown() {
        let db = Arc::new(Database::new_in_memory());
        let id = db.create_environment("A").unwrap();
        let persistence = EnvironmentPersistence::with_debounce(db.clone(), Duration::ZERO);
        wait(edit(&persistence, id, "saved")).unwrap();
        assert_eq!(
            db.load_environments().unwrap()[0].variables[0].value,
            "saved"
        );
    }

    #[test]
    fn immediate_shutdown_preserves_edit_when_database_is_reopened() {
        static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "poopman-environment-shutdown-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let db = Arc::new(Database::open_test_file(&path));
        let id = db.create_environment("A").unwrap();
        let persistence = queue(db.clone());
        drop(edit(&persistence, id, "survives relaunch"));
        wait(persistence.shutdown()).unwrap();
        let reopened = Database::open_test_file(&path);
        assert_eq!(
            reopened.load_environments().unwrap()[0].variables[0].value,
            "survives relaunch"
        );
        drop(reopened);
        drop(persistence);
        drop(db);
        // Database threads release their file handles asynchronously.
        let _ = std::fs::remove_file(path);
    }
}
