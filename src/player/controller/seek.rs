use super::Controller;
use crate::player::{Backend, Handle, SeekStatus, seek_target};
use anyhow::{Context, Result};
use std::future::pending;
use std::time::Duration;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::{Instant, timeout};

pub(super) const SEEK_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct SeekJob {
    task: JoinHandle<Result<Duration>>,
    track_id: u64,
    target: Duration,
    next: Option<Duration>,
}

impl Drop for SeekJob {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn seek_result(job: &mut Option<SeekJob>) -> Result<Result<Duration>, JoinError> {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

impl<B: Backend> Controller<B> {
    pub(super) fn seek(&mut self, track_id: u64, position: Duration) {
        if !self.states.borrow().can_seek()
            || self.track.as_ref().is_none_or(|track| track.id != track_id)
        {
            return self.sync();
        }
        let Some(current) = &mut self.track else {
            return;
        };
        let Some(target) = seek_target(position, current.info.duration) else {
            return;
        };
        if self.seek_job.is_none() && target == current.position {
            return self.sync();
        }
        if let Some(job) = &mut self.seek_job {
            job.next = (target != job.target).then_some(target);
        } else {
            let handle = current.handle.clone();
            self.seek_job = Some(SeekJob {
                track_id,
                target,
                next: None,
                task: tokio::spawn(async move {
                    timeout(SEEK_TIMEOUT, handle.seek(target)).await
                        .context("Seeking timed out. This stream cannot be sought again; use Play URL to reopen it.")?
                        .context("This stream could not seek. Use Play URL to reopen it.")
                }),
            });
        }
        current.seek = SeekStatus::Pending(target);
        self.sync();
    }

    pub(super) fn finish_seek(&mut self, result: Result<Result<Duration>, JoinError>) {
        let Some(job) = self.seek_job.take() else {
            return;
        };
        let Some(current) = &mut self.track else {
            return;
        };
        if job.track_id != current.id {
            return;
        }
        match result
            .context("Seek task failed.")
            .and_then(|result| result)
        {
            Ok(actual) => {
                current.last_info = Instant::now();
                current.position = super::whole_seconds(actual);
                current.seek = SeekStatus::Complete(actual);
                current.seek_serial = current.seek_serial.wrapping_add(1);
                let state = current.state(None);
                self.emit(state);
                if let Some(next) = job.next {
                    self.seek(job.track_id, next);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Seek failed");
                current.seek = SeekStatus::Failed(error.to_string());
                self.sync();
            }
        }
    }
}
