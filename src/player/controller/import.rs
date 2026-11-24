use super::Controller;
use crate::player::Backend;
use crate::playlist::{Error, ImportStatus, Playlist};
use crate::queue::full_message;
use crate::youtube::playlist_url;
use anyhow::Result;
use std::future::pending;
use tokio::task::{JoinError, JoinHandle};

/// The import owns its task even when the controller future is dropped abruptly.
pub(super) struct ImportJob {
    task: JoinHandle<Result<Playlist>>,
}

impl Drop for ImportJob {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn import_result(
    job: &mut Option<ImportJob>,
) -> Result<Result<Playlist>, JoinError> {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

impl<B: Backend> Controller<B> {
    pub(super) fn start_import(&mut self, value: &str) {
        if self.import_job.is_some() {
            return self.sync();
        }
        match playlist_url(value) {
            Err(error) => self.import = ImportStatus::Failed(error.to_string()),
            Ok(_) if self.queue.is_full() => {
                self.import = ImportStatus::Failed(full_message("Remove entries and try again."));
            }
            Ok(url) => {
                let backend = self.backend.clone();
                self.import_job = Some(ImportJob {
                    task: tokio::spawn(async move { backend.playlist(url).await }),
                });
                self.import = ImportStatus::Importing;
            }
        }
        self.sync();
    }

    pub(super) async fn cancel_import(&mut self) {
        if let Some(mut job) = self.import_job.take() {
            job.task.abort();
            let _ = (&mut job.task).await;
            self.import = ImportStatus::Cancelled;
        }
    }

    pub(super) async fn cancel_import_command(&mut self) {
        self.cancel_import().await;
        self.sync();
    }

    pub(super) fn finish_import(&mut self, result: Result<Result<Playlist>, JoinError>) {
        if self.import_job.take().is_none() {
            return;
        }
        match result {
            Ok(Ok(playlist)) => {
                let total = playlist.entries.len();
                let added = self.queue.append(
                    playlist
                        .entries
                        .into_iter()
                        .map(|entry| (entry.url, entry.title)),
                );
                self.import = if added == 0 {
                    ImportStatus::Failed(full_message("Remove entries and try again."))
                } else {
                    ImportStatus::Finished {
                        added,
                        unavailable: playlist.unavailable,
                        omitted: total - added,
                        limited: playlist.limited,
                    }
                };
                if added > 0 {
                    self.queue_changed();
                } else {
                    self.sync();
                }
            }
            error => {
                tracing::debug!(?error, "Playlist import failed");
                if matches!(&error, Ok(Err(error)) if error.downcast_ref::<Error>() == Some(&Error::MissingComponents))
                {
                    self.components_request = self.components_request.wrapping_add(1);
                    self.import = ImportStatus::Ready;
                    self.sync();
                    return;
                }
                self.import = ImportStatus::Failed(match error {
                    Ok(Err(error)) => error.downcast_ref::<Error>().map_or_else(
                        || "Playlist unavailable. Check the public link and try again.".into(),
                        ToString::to_string,
                    ),
                    _ => "Playlist import failed. Try again.".into(),
                });
                self.sync();
            }
        }
    }
}
