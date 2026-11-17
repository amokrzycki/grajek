use super::{Backend, Controller, JoinError, JoinHandle, pending, watch};
use crate::{
    config::video_url,
    player::{Command, SearchAction, SearchPhase, SearchState},
    queue::QUEUE_LIMIT,
    search::{Error, Results, TIMEOUT, query},
};
use std::time::Duration;
use tokio::time::Instant;

type Completion = std::result::Result<std::result::Result<Results, Error>, JoinError>;

#[derive(Default)]
pub(super) struct Search {
    pub state: SearchState,
    pub job: Option<Job>,
    pending: Option<Request>,
}

struct Request {
    id: u64,
    query: String,
    deadline: Instant,
}

pub(super) struct Job {
    id: u64,
    cancel: watch::Sender<bool>,
    task: JoinHandle<std::result::Result<Results, Error>>,
    deadline: Instant,
    cleanup_deadline: Option<Instant>,
}

impl Job {
    fn cancel(&mut self) {
        if !*self.cancel.borrow() {
            self.cancel.send_replace(true);
            self.cleanup_deadline = Some(Instant::now() + Duration::from_secs(2));
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}

impl Search {
    pub fn deadline(&self) -> Option<Instant> {
        let work = self.job.as_ref().and_then(|job| {
            if *job.cancel.borrow() {
                job.cleanup_deadline
            } else {
                Some(job.deadline)
            }
        });
        work.into_iter()
            .chain(self.pending.as_ref().map(|request| request.deadline))
            .min()
    }
}

pub(super) async fn result(job: &mut Option<Job>) -> Completion {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

impl<B: Backend> Controller<B> {
    pub(super) fn submit_search(&mut self, id: u64, text: String) {
        if id <= self.search.state.id {
            return self.sync();
        }
        let normalized = query(&text);
        if normalized
            .as_ref()
            .is_ok_and(|query| query == &self.search.state.query)
            && self.search.state.phase == SearchPhase::Loading
        {
            return self.sync();
        }
        if let Some(job) = &mut self.search.job {
            job.cancel();
        }
        self.search.pending = None;
        self.search.state = SearchState {
            id,
            query: normalized.clone().unwrap_or(text),
            phase: SearchPhase::Loading,
            status: None,
        };
        if self
            .search
            .job
            .as_ref()
            .is_some_and(|job| *job.cancel.borrow() && job.cleanup_deadline.is_none())
        {
            self.search.state.phase = SearchPhase::Failed(Error::Timeout);
            self.search.state.status =
                Some("Search cleanup is still running. Wait before searching again.".into());
            return self.sync();
        }
        match normalized {
            Ok(query) => {
                self.search.pending = Some(Request {
                    id,
                    query,
                    deadline: Instant::now() + TIMEOUT,
                });
                self.start_search();
            }
            Err(error) => self.search.state.phase = SearchPhase::Failed(error),
        }
        self.sync();
    }

    fn start_search(&mut self) {
        if self.search.job.is_some() {
            return;
        }
        let Some(request) = self.search.pending.take() else {
            return;
        };
        if request.deadline <= Instant::now() {
            self.apply_search_result(request.id, Err(Error::Timeout));
            return;
        }
        let backend = self.backend.clone();
        let (cancel, cancelled) = watch::channel(false);
        self.search.job = Some(Job {
            id: request.id,
            deadline: request.deadline,
            cleanup_deadline: None,
            cancel,
            task: tokio::spawn(async move { backend.search(request.query, cancelled).await }),
        });
    }

    pub(super) fn cancel_search(&mut self, id: u64) {
        if id != self.search.state.id {
            return self.sync();
        }
        self.search.pending = None;
        if let Some(job) = &mut self.search.job {
            job.cancel();
        }
        self.search.state.phase = SearchPhase::Idle;
        self.search.state.status = None;
        self.sync();
    }

    pub(super) fn apply_search_result(
        &mut self,
        id: u64,
        result: std::result::Result<Results, Error>,
    ) {
        if self.search.state.id != id || self.search.state.phase != SearchPhase::Loading {
            return;
        }
        self.search.state.phase = match result {
            Ok(results) if results.items.is_empty() => SearchPhase::Empty {
                rejected: results.rejected,
            },
            Ok(results) => SearchPhase::Results(results),
            Err(error) => SearchPhase::Failed(error),
        };
        self.sync();
    }

    pub(super) fn finish_search(&mut self, result: Completion) {
        let Some(job) = self.search.job.take() else {
            return;
        };
        let id = job.id;
        let result = if Instant::now() >= job.deadline {
            Err(Error::Timeout)
        } else {
            result.unwrap_or(Err(Error::Tool))
        };
        self.apply_search_result(id, result);
        self.start_search();
    }

    pub(super) fn search_expired(&mut self) {
        let now = Instant::now();
        if self
            .search
            .pending
            .as_ref()
            .is_some_and(|request| request.deadline <= now)
        {
            self.search.pending = None;
            self.apply_search_result(self.search.state.id, Err(Error::Timeout));
        }
        if let Some(job) = &mut self.search.job {
            if job.cleanup_deadline.is_some_and(|deadline| deadline <= now) {
                job.cleanup_deadline = None;
                self.search.pending = None;
                self.search.state.phase = SearchPhase::Failed(Error::Timeout);
                self.search.state.status =
                    Some("Search cleanup is still running. Wait before searching again.".into());
                tracing::error!(
                    search_id = job.id,
                    "Search cleanup exceeded two seconds; retaining worker until reaped"
                );
            } else if !*job.cancel.borrow() && job.deadline <= now {
                let id = job.id;
                job.cancel();
                self.apply_search_result(id, Err(Error::Timeout));
            }
        }
        self.sync();
    }

    pub(super) async fn join_search(&mut self) {
        self.search.pending = None;
        if let Some(mut job) = self.search.job.take() {
            job.cancel();
            let _ = (&mut job.task).await;
        }
    }

    pub(super) async fn search_action(&mut self, id: u64, video_id: &str, action: SearchAction) {
        if id != self.search.state.id {
            return self.sync();
        }
        let SearchPhase::Results(results) = &self.search.state.phase else {
            return self.sync();
        };
        let Some(url) = results
            .items
            .iter()
            .find(|item| item.id() == video_id)
            .and_then(|item| video_url(&item.url).ok())
        else {
            return self.sync();
        };
        let state = self.view(self.states.borrow().clone());
        let refusal = match action {
            SearchAction::Play if !state.can_play() => {
                Some("Playback unavailable. Restore Discord and try again.")
            }
            SearchAction::Enqueue { .. } if !state.can_enqueue() => {
                Some("Queueing unavailable. Restore Discord and try again.")
            }
            SearchAction::Enqueue { .. } if state.queue.len() >= QUEUE_LIMIT => {
                Some("Queue full (100 tracks). Remove an entry and try again.")
            }
            _ => None,
        };
        if let Some(message) = refusal {
            self.search.state.status = Some(message.into());
            if let SearchAction::Enqueue { id, .. } = action {
                self.answer(id, None);
            }
            return self.sync();
        }
        self.search.state.phase = SearchPhase::Idle;
        self.search.state.status = None;
        match action {
            SearchAction::Play => Box::pin(self.command(Command::Play(url))).await,
            SearchAction::Enqueue { id, placement } => {
                let item_id = self.enqueue(&url, placement);
                self.answer(id, item_id);
            }
        }
        self.sync();
    }
}
