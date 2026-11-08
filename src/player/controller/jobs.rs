use super::{Active, Controller, HistoryEntry, Resolved};
use crate::queue::QueueItem;
use crate::{
    media,
    player::{Backend, Phase, volume_factor},
};
use anyhow::{Context as _, Result};
use std::future::pending;
use std::time::Duration;
use tokio::time::timeout;
use tokio::{
    sync::oneshot,
    task::{JoinError, JoinHandle},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    Direct,
    Queue,
}

#[derive(Clone)]
pub(super) struct Request {
    pub url: String,
    pub origin: Origin,
}

#[derive(Clone)]
pub(super) enum Start {
    Track(Request),
    Queue(Vec<QueueItem>),
    Rotate(HistoryEntry),
}

/// The input and correctly typed task live together until cancellation or completion.
pub(super) enum Job {
    Preflight {
        task: JoinHandle<Result<()>>,
        start: Start,
    },
    Prepare {
        task: JoinHandle<Result<Resolved>>,
        request: Request,
        connected: Option<oneshot::Receiver<()>>,
        voice_ready: bool,
    },
    Rejoin {
        task: JoinHandle<Result<()>>,
        attempt: u8,
    },
    Disconnect {
        task: JoinHandle<Result<()>>,
        manual: bool,
    },
}

impl Job {
    pub fn preparing(&self) -> bool {
        matches!(self, Self::Preflight { .. } | Self::Prepare { .. })
    }

    pub fn manual_disconnect(&self) -> bool {
        matches!(self, Self::Disconnect { manual: true, .. })
    }

    fn abort(&self) {
        match self {
            Self::Prepare { task, .. } => task.abort(),
            Self::Preflight { task, .. }
            | Self::Rejoin { task, .. }
            | Self::Disconnect { task, .. } => task.abort(),
        }
    }

    pub async fn cancel(mut self) {
        self.abort();
        match &mut self {
            Self::Prepare { task, .. } => {
                let _ = task.await;
            }
            Self::Preflight { task, .. }
            | Self::Rejoin { task, .. }
            | Self::Disconnect { task, .. } => {
                let _ = task.await;
            }
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.abort();
    }
}

pub(super) enum Event {
    Checked {
        start: Start,
        result: Result<()>,
    },
    VoiceReady(bool),
    Prepared {
        request: Request,
        voice_ready: bool,
        result: Result<Resolved>,
    },
    Rejoined {
        attempt: u8,
        result: Result<()>,
    },
    Disconnected(Result<()>),
}

fn completed<T>(result: Result<Result<T>, JoinError>) -> Result<T> {
    result
        .context("Playback job task failed.")
        .and_then(|result| result)
}

pub(super) async fn event(slot: &mut Option<Job>) -> Event {
    let Some(job) = slot.as_mut() else {
        return pending().await;
    };
    let event = match job {
        Job::Preflight { task, start } => Event::Checked {
            start: start.clone(),
            result: completed(task.await),
        },
        Job::Prepare {
            task,
            request,
            connected,
            voice_ready,
        } => {
            if let Some(receiver) = connected {
                tokio::select! {
                    biased;
                    reached = receiver => {
                        *voice_ready = reached.is_ok();
                        *connected = None;
                        return Event::VoiceReady(*voice_ready);
                    }
                    result = task => Event::Prepared { request: request.clone(), voice_ready: *voice_ready, result: completed(result) },
                }
            } else {
                Event::Prepared {
                    request: request.clone(),
                    voice_ready: *voice_ready,
                    result: completed(task.await),
                }
            }
        }
        Job::Rejoin { task, attempt } => Event::Rejoined {
            attempt: *attempt,
            result: completed(task.await),
        },
        Job::Disconnect { task, .. } => Event::Disconnected(completed(task.await)),
    };
    *slot = None;
    event
}

impl<B: Backend> Controller<B> {
    pub(super) async fn finish_job(&mut self, event: Event) {
        match event {
            Event::Checked { start, result } => self.finish_preflight(start, result),
            Event::VoiceReady(reached) => {
                if reached && self.states.borrow().phase == Phase::Connecting {
                    self.publish(Phase::Resolving, None);
                }
            }
            Event::Prepared {
                request,
                result: Ok(resolved),
                ..
            } => {
                if !self.gateway() {
                    self.publish(Phase::Error, Some("Discord destination is no longer available. Select it again before playing.".into()));
                    return;
                }
                let queued = request.origin == Origin::Queue;
                let info = resolved.info.clone();
                match timeout(
                    Duration::from_secs(2),
                    self.backend.play(resolved, volume_factor(self.volume)),
                )
                .await
                {
                    Ok(Ok((handle, outcome))) => {
                        self.track_id = self.track_id.wrapping_add(1);
                        let mut current = Active::new(handle, outcome, info, request);
                        current.id = self.track_id;
                        let state = current.state(None);
                        self.track = Some(current);
                        self.emit(state);
                    }
                    Ok(Err(error)) => {
                        tracing::error!(error = %format!("{error:#}"), "Could not start audio playback");
                        self.media_failure_for(
                            "Could not start audio playback. Try again.",
                            queued,
                        );
                    }
                    Err(_) => self.media_failure_for(
                        "Audio playback startup timed out. Press Next or Play URL.",
                        queued,
                    ),
                }
            }
            Event::Prepared {
                request,
                voice_ready,
                result: Err(error),
            } => {
                tracing::error!(error = %format!("{error:#}"), "Audio request failed");
                if error
                    .downcast_ref::<media::components::MissingComponents>()
                    .is_some()
                {
                    self.components_request = self.components_request.wrapping_add(1);
                    self.failure_notice = None;
                    self.scheduled = None;
                    self.publish(Phase::Ready, None);
                    return;
                }
                self.media_failure_for(
                    &media::user_error(&error),
                    request.origin == Origin::Queue && voice_ready,
                );
                self.backend.recover().await;
            }
            Event::Rejoined { attempt, result } => self.finish_rejoin(attempt, result).await,
            Event::Disconnected(result) => {
                if let Err(error) = result {
                    tracing::warn!(%error, "Voice disconnect failed");
                    self.publish(Phase::Error, Some("Could not leave Discord voice. Check the network and try Leave voice again.".into()));
                } else {
                    let error = self.health.borrow().error();
                    self.publish(
                        if error.is_some() {
                            Phase::Error
                        } else {
                            Phase::Ready
                        },
                        error,
                    );
                }
            }
        }
    }

    async fn finish_rejoin(&mut self, attempt: u8, result: Result<()>) {
        self.rejoin_attempts = attempt;
        if result.is_ok() && self.health.borrow().voice {
            self.rejoin_attempts = 0;
            self.publish(
                Phase::Ready,
                Some("Discord voice restored. Press Next or Play URL to start playback.".into()),
            );
        } else {
            let retry = {
                let health = self.health.borrow();
                // Songbird may return the join error before its disconnect callback.
                health.voice_retry
                    || (health.error().is_none()
                        && result.as_ref().is_err_and(crate::discord::retryable_join))
            };
            self.backend.recover().await;
            if retry && self.gateway() {
                self.schedule_rejoin();
            } else {
                self.publish(Phase::Error, Some("Could not restore Discord voice. Check the network and permissions, then press Next or Play URL.".into()));
            }
        }
    }
}
