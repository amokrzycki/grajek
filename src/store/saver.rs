use std::path::PathBuf;

use tokio::{sync::watch, task::JoinHandle};

use super::{file::write, saved::Saved};

pub struct Saver {
    pub task: JoinHandle<()>,
    pub status: watch::Receiver<Option<&'static str>>,
}

/// The instance's only writer. It saves the latest snapshot and skips intermediate ones, so a
/// slow write can never land after, and overwrite, newer state. It exits (after saving
/// the final snapshot) when the sender is dropped.
pub fn spawn_saver(path: PathBuf, mut snapshots: watch::Receiver<Saved>) -> Saver {
    let (status, updates) = watch::channel(None);
    let task = tokio::spawn(async move {
        while snapshots.changed().await.is_ok() {
            let saved = snapshots.borrow_and_update().clone();
            let target = path.clone();
            let warning = match tokio::task::spawn_blocking(move || write(&target, &saved)).await {
                Ok(Ok(())) => None,
                result => {
                    tracing::warn!(?result, "Could not save application state");
                    Some(
                        "Could not save local data and settings. Check disk space and file permissions; changes may be lost when Grajek closes.",
                    )
                }
            };
            status.send_if_modified(|previous| {
                if *previous == warning {
                    return false;
                }
                *previous = warning;
                true
            });
        }
    });
    Saver {
        task,
        status: updates,
    }
}
