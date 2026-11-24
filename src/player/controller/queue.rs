use super::Controller;
use crate::history;
use crate::player::Backend;
use crate::queue::{Full, Placement, full_notice};
use crate::store::SavedQueueItem;
use crate::youtube::{safe_title, video_url};
use anyhow::{Context, Result};
use std::future::pending;
use std::time::Duration;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::timeout;

pub(super) const TITLE_TIMEOUT: Duration = Duration::from_secs(20);

/// One title subprocess at a time, independent of playback and playlist import.
pub(super) struct TitleJob {
    url: String,
    ids: Vec<u64>,
    task: JoinHandle<Result<Option<String>>>,
}

pub(super) async fn title_result(
    job: &mut Option<TitleJob>,
) -> Result<Result<Option<String>>, JoinError> {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

impl<B: Backend> Controller<B> {
    pub(super) fn queue_changed(&mut self) {
        self.saved.queue = if self.saved.remember_queue {
            self.queue
                .snapshot()
                .into_iter()
                .map(|item| SavedQueueItem {
                    url: item.url,
                    title: item.title.as_deref().and_then(safe_title),
                })
                .collect()
        } else {
            Vec::new()
        };
        self.persist();
        self.sync();
    }

    fn known_title(&self, url: &str) -> Option<String> {
        history::title_of(&self.saved.history, url)
            .or_else(|| self.queue.title_of(url))
            .or_else(|| {
                self.track
                    .as_ref()
                    .filter(|track| track.request.url == url)
                    .and_then(|track| track.info.title.clone())
            })
    }

    pub(super) async fn refresh_titles(&mut self) {
        let mut changed = false;
        for item in self
            .queue
            .snapshot()
            .iter()
            .filter(|item| item.title.is_none())
        {
            if let Some(title) = self.known_title(&item.url) {
                self.queue.resolve_titles(&[item.id], Some(&title));
                changed = true;
            }
        }
        if changed {
            self.queue_changed();
        }
        let items = self.queue.snapshot();
        if let Some(job) = &mut self.title_job {
            job.ids.retain(|id| {
                items
                    .iter()
                    .any(|item| item.id == *id && item.title.is_none())
            });
            if job.ids.is_empty() {
                if let Some(job) = self.title_job.take() {
                    job.task.abort();
                    let _ = job.task.await;
                }
            } else {
                for item in &items {
                    if item.url == job.url && item.title.is_none() && !job.ids.contains(&item.id) {
                        job.ids.push(item.id);
                    }
                }
            }
        }
        if self.title_job.is_some() {
            return;
        }
        let Some(item) = items
            .iter()
            .find(|item| item.title.is_none() && !item.title_unavailable)
        else {
            return;
        };
        let url = item.url.clone();
        let ids = items
            .iter()
            .filter(|item| item.url == url && item.title.is_none())
            .map(|item| item.id)
            .collect();
        let backend = self.backend.clone();
        let request = url.clone();
        self.title_job = Some(TitleJob {
            url,
            ids,
            task: tokio::spawn(async move {
                timeout(TITLE_TIMEOUT, backend.title(request))
                    .await
                    .context("Title lookup timed out.")?
            }),
        });
    }

    pub(super) fn finish_title(&mut self, result: Result<Result<Option<String>>, JoinError>) {
        let Some(job) = self.title_job.take() else {
            return;
        };
        let title = match result {
            Ok(Ok(title)) => title.and_then(|title| safe_title(&title)),
            result => {
                tracing::debug!(?result, "Queue title lookup failed");
                None
            }
        };
        self.queue.resolve_titles(&job.ids, title.as_deref());
        self.queue_changed();
    }

    pub(super) async fn cancel_titles(&mut self) {
        if let Some(job) = self.title_job.take() {
            job.task.abort();
            let _ = job.task.await;
        }
    }

    pub(super) fn enqueue(&mut self, value: &str, placement: Placement) -> Option<u64> {
        match video_url(value) {
            Ok(url) => {
                let title = self.known_title(&url);
                match self.queue.insert(url, title, placement) {
                    Ok(id) => {
                        self.queue_changed();
                        return Some(id);
                    }
                    Err(Full) => {
                        self.report(full_notice());
                    }
                }
            }
            Err(error) => self.report(error.to_string()),
        }
        None
    }

    pub(super) fn answer(&mut self, request_id: u64, item_id: Option<u64>) {
        self.enqueue_result = Some(crate::player::EnqueueResult {
            request_id,
            item_id,
        });
    }

    pub(super) fn enqueue_command(&mut self, request_id: u64, value: &str, placement: Placement) {
        let item_id = self.enqueue(value, placement);
        self.answer(request_id, item_id);
        self.sync();
    }

    pub(super) fn move_to_next(&mut self, request_id: u64, id: u64) {
        let moved = self.queue.move_to_front(id).then_some(id);
        self.answer(request_id, moved);
        self.queue_changed();
    }

    pub(super) fn dequeue(&mut self, id: u64) {
        self.queue.remove(id);
        self.queue_changed();
    }

    pub(super) fn shift(&mut self, id: u64, up: bool) {
        self.queue.shift(id, up);
        self.queue_changed();
    }

    pub(super) async fn clear_queue(&mut self) {
        self.cancel_import().await;
        self.queue.clear();
        self.queue_changed();
    }
}
