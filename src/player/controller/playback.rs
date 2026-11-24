use super::jobs::{Job, Origin, Request, Start};
use super::{Backend, Controller};
use crate::history::{self, HistoryEntry};
use crate::media::TrackInfo;
use crate::player::{Handle, Outcome, Phase, SeekStatus, State, VolumeRequest, volume_factor};
use crate::preferences::{RepeatMode, clamp_volume};
use crate::youtube::video_url;
use songbird::tracks::{PlayMode, TrackResult, TrackState};
use std::future::pending;
use std::time::Duration;
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

/// How long a track may take to start advancing before it is reported as failed.
const START_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const UNRESPONSIVE_TIMEOUT: Duration = Duration::from_secs(15);

/// What one `get_info()` poll means for the current track.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observed {
    Idle,
    Changed,
    Done(Option<String>),
}

/// The one live track. Its event mailbox belongs to it, so a replaced track can never
/// report an outcome for its successor.
pub(super) struct Active<H> {
    pub(super) id: u64,
    pub(super) seek: SeekStatus,
    pub(super) seek_serial: u64,
    pub(super) handle: H,
    pub(super) outcome: watch::Receiver<Option<Outcome>>,
    pub(super) info: TrackInfo,
    pub(super) request: Request,
    started: Instant,
    pub(super) last_info: Instant,
    /// Position has advanced at least once; Playing is not claimed before this.
    pub(super) confirmed: bool,
    /// The video has been added to the history (once, when playback begins).
    pub(super) recorded: bool,
    pub(super) paused: bool,
    pub(super) position: Duration,
}

impl<H: Handle> Active<H> {
    pub(super) fn new(
        handle: H,
        outcome: watch::Receiver<Option<Outcome>>,
        info: TrackInfo,
        request: Request,
    ) -> Self {
        Self {
            id: 0,
            seek: SeekStatus::Ready,
            seek_serial: 0,
            handle,
            outcome,
            info,
            request,
            started: Instant::now(),
            last_info: Instant::now(),
            confirmed: false,
            recorded: false,
            paused: false,
            position: Duration::ZERO,
        }
    }

    pub(super) fn state(&self, error: Option<String>) -> State {
        let phase = if self.paused {
            Phase::Paused
        } else if self.confirmed {
            Phase::Playing
        } else {
            Phase::Resolving
        };
        State {
            phase,
            error,
            available: true,
            track: Some(self.info.clone()),
            position: self.position,
            track_id: self.id,
            seek: self.seek.clone(),
            seek_serial: self.seek_serial,
            ..State::default()
        }
    }

    fn entry(&self) -> HistoryEntry {
        HistoryEntry {
            url: self.request.url.clone(),
            title: self.info.title.clone(),
            duration_secs: self.info.duration.map(|duration| duration.as_secs()),
        }
    }

    fn stalled(&self) -> bool {
        !self.confirmed && self.started.elapsed() > START_TIMEOUT
    }

    pub(super) fn observe(&mut self, info: &TrackState) -> Observed {
        match &info.playing {
            PlayMode::Errored(error) => return Observed::Done(Some(error.to_string())),
            PlayMode::End => return Observed::Done(None),
            PlayMode::Stop => {
                return Observed::Done(Some(
                    "Audio track stopped unexpectedly. Press Next or Play URL.".into(),
                ));
            }
            PlayMode::Play if self.paused || info.position.is_zero() => return Observed::Idle,
            PlayMode::Pause if !self.paused => return Observed::Idle,
            _ => {}
        }
        let position = super::whole_seconds(info.position);
        let changed = !self.confirmed || position != self.position;
        self.confirmed = true;
        self.position = position;
        if changed {
            Observed::Changed
        } else {
            Observed::Idle
        }
    }
}

pub(super) async fn track_outcome<H>(track: &mut Option<Active<H>>) -> bool {
    match track {
        Some(current) => current.outcome.changed().await.is_ok(),
        None => pending().await,
    }
}

impl<B: Backend> Controller<B> {
    pub(super) async fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel().await;
        }
    }

    pub(super) fn stop_track(&mut self) {
        self.seek_job = None;
        if let Some(track) = self.track.take()
            && let Err(error) = track.handle.stop()
        {
            tracing::debug!(%error, "Track already finished");
        }
    }

    pub(super) fn fail(&mut self, message: &str) {
        self.stop_track();
        self.publish(Phase::Error, Some(message.into()));
    }

    /// Only one request and one track can exist because everything runs through this single path.
    async fn start(&mut self, start: Start) {
        self.left_voice = false;
        self.scheduled = None;
        self.cancel_job().await;
        self.stop_track();
        self.backend.clear_error();
        let backend = self.backend.clone();
        self.job = Some(Job::Preflight {
            task: tokio::spawn(async move { backend.check_tools().await }),
            start,
        });
        self.publish(Phase::Connecting, None);
    }

    pub(super) fn prepare(&mut self, request: Request) {
        let (connected_tx, connected) = oneshot::channel();
        let backend = self.backend.clone();
        let url = request.url.clone();
        self.job = Some(Job::Prepare {
            task: tokio::spawn(async move { backend.prepare(url, connected_tx).await }),
            connected: Some(connected),
            request,
            voice_ready: false,
        });
    }

    /// Called exactly once per track that ended on its own: the next queued track, or Ready.
    pub(super) async fn advance(&mut self) {
        let mut selected = match &self.job {
            Some(Job::Preflight {
                start: Start::Queue(selected),
                ..
            }) => selected.clone(),
            _ => Vec::new(),
        };
        let next = self
            .queue
            .snapshot()
            .into_iter()
            .find(|item| !selected.iter().any(|pending| pending.id == item.id));
        if let Some(next) = next {
            selected.push(next);
            self.start(Start::Queue(selected)).await;
        } else if self.job.is_none() {
            self.publish(Phase::Ready, None);
        } else {
            self.sync();
        }
    }

    /// Only a confirmed natural completion can recycle media. Errors and manual
    /// cancellation never enter this path; each successor gets fresh extraction.
    pub(super) async fn natural_end(&mut self) {
        let Some(current) = self.track.as_ref() else {
            return;
        };
        if self.saved.repeat != RepeatMode::Off && !current.confirmed {
            self.media_failure(
                "Audio ended before playback was confirmed. Press Next or Play URL.",
            );
            return;
        }
        let completed = current.entry();
        let request = current.request.clone();
        self.stop_track();
        match self.saved.repeat {
            RepeatMode::Track => self.start(Start::Track(request)).await,
            RepeatMode::Queue if request.origin == Origin::Queue => {
                let next = self.queue.snapshot().into_iter().next();
                self.start(Start::Rotate { completed, next }).await;
            }
            RepeatMode::Off | RepeatMode::Queue => self.advance().await,
        }
    }

    pub(super) async fn play_command(&mut self, value: &str) {
        if self.preparing() || self.manual_disconnect() || !self.gateway() {
            return self.sync();
        }
        match video_url(value) {
            Ok(url) => {
                self.failure_notice = None;
                self.rejoin_attempts = 0;
                self.start(Start::Track(Request {
                    url,
                    origin: Origin::Direct,
                }))
                .await;
            }
            // A bad URL must not disturb the track that is still playing or paused.
            Err(error) => self.report(error.to_string()),
        }
    }

    pub(super) async fn next_command(&mut self) {
        self.failure_notice = None;
        self.rejoin_attempts = 0;
        if !self.gateway() {
            return self.sync();
        }
        if self.states.borrow().can_next() {
            self.advance().await;
        } else {
            self.sync();
        }
    }

    fn preparing(&self) -> bool {
        self.job.as_ref().is_some_and(Job::preparing)
    }

    pub(super) fn manual_disconnect(&self) -> bool {
        self.job.as_ref().is_some_and(Job::manual_disconnect)
    }

    /// Applies Pause/Resume. The controller is the authority: it always answers with a state,
    /// so a stale or duplicate click can never leave the GUI waiting or showing the wrong button.
    pub(super) fn set_paused(&mut self, paused: bool) {
        self.set_paused_with(paused, |handle| {
            if paused {
                handle.pause()
            } else {
                handle.resume()
            }
        });
    }

    pub(super) fn set_paused_with(
        &mut self,
        paused: bool,
        send: impl FnOnce(&B::Handle) -> TrackResult<()>,
    ) {
        let Some(current) = self.track.as_mut() else {
            return self.sync();
        };
        if !current.confirmed || current.paused == paused {
            let state = current.state(None);
            return self.emit(state);
        }
        match send(&current.handle) {
            Ok(()) => {
                current.paused = paused;
                let state = current.state(None);
                self.emit(state);
            }
            Err(error) => {
                tracing::warn!(%error, paused, "Track control failed");
                let action = if paused { "pause" } else { "resume" };
                self.fail(&format!(
                    "Could not {action} playback. The track is no longer active."
                ));
            }
        }
    }

    /// Applies without restarting the track.
    pub(super) fn set_volume(&mut self, request: VolumeRequest) {
        let percent = clamp_volume(request.percent);
        self.volume_seq = request.seq;
        if let Some(current) = &self.track
            && let Err(error) = current.handle.set_volume(volume_factor(percent))
        {
            // The old volume stays: nothing was applied, so none may be claimed.
            tracing::warn!(%error, "Volume control failed");
            return self.fail("Could not change the volume. The track is no longer active.");
        }
        self.volume = percent;
        if request.commit && self.saved.volume != percent {
            self.saved.volume = percent;
            self.persist();
        }
        self.sync();
    }

    /// `alive` is false when the track's event senders disappeared without reporting an
    /// outcome. Returns true when the track ended on its own.
    pub(super) fn handle_outcome(&mut self, alive: bool) -> bool {
        let Some(current) = self.track.as_mut() else {
            return false;
        };
        if !alive {
            self.media_failure("Audio driver terminated unexpectedly. Try again.");
            return false;
        }
        let outcome = current.outcome.borrow_and_update().clone();
        match outcome {
            Some(Outcome::End) => true,
            Some(Outcome::Error(error)) => {
                self.media_failure(&error);
                false
            }
            None => false,
        }
    }

    pub(super) fn apply(&mut self, reading: Option<TrackResult<TrackState>>) -> bool {
        let Some(current) = self.track.as_mut() else {
            return false;
        };
        let failure = match reading {
            Some(Ok(info)) => {
                current.last_info = Instant::now();
                match current.observe(&info) {
                    Observed::Changed => {
                        let state = current.state(None);
                        // History records a video once it is really playing, not when yt-dlp succeeds.
                        let entry = (!current.recorded).then(|| {
                            current.recorded = true;
                            current.entry()
                        });
                        self.failures = 0;
                        if let Some(entry) = entry {
                            history::record(&mut self.saved.history, entry);
                            self.persist();
                        }
                        self.emit(state);
                        None
                    }
                    Observed::Done(error) => {
                        let ended = error.is_none();
                        if let Some(error) = error {
                            self.media_failure(&error);
                        }
                        return ended;
                    }
                    Observed::Idle => current
                        .stalled()
                        .then_some("Audio playback did not start. Try another video."),
                }
            }
            Some(Err(error)) => {
                tracing::debug!(%error, "Track handle finished");
                // Allow terminal events to arrive; a dead driver cannot hold the UI forever.
                (current.started.elapsed() > START_TIMEOUT)
                    .then_some("Audio track terminated unexpectedly. Try again.")
            }
            None => (current.last_info.elapsed() >= UNRESPONSIVE_TIMEOUT)
                .then_some("Audio driver stopped responding. Try again."),
        };
        if let Some(message) = failure {
            self.media_failure(message);
        }
        false
    }
}
