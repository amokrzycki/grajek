use anyhow::{Context, Result};
use songbird::tracks::{PlayMode, TrackResult, TrackState};
use std::{
    future::pending,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
    time::{interval, timeout},
};

use super::{Backend, Command, Handle, Outcome, Phase, State, VolumeRequest, volume_factor};
use crate::{
    config::video_url,
    discord::Health,
    media::{self, Resolved, TrackInfo},
    queue::{Full, QUEUE_LIMIT, Queue},
    store::{HistoryEntry, Saved},
};

/// How long a track may take to start advancing before it is reported as failed.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// What one `get_info()` poll means for the current track.
#[derive(Debug, PartialEq, Eq)]
enum Observed {
    Idle,
    Changed,
    Done(Option<String>),
}

/// The one live track. Its event mailbox belongs to it, so a replaced track can never
/// report an outcome for its successor.
struct Active<H> {
    handle: H,
    outcome: watch::Receiver<Option<Outcome>>,
    info: TrackInfo,
    url: String,
    started: Instant,
    /// Position has advanced at least once; Playing is not claimed before this.
    confirmed: bool,
    /// The video has been added to the history (once, when playback begins).
    recorded: bool,
    paused: bool,
    position: Duration,
}

impl<H: Handle> Active<H> {
    fn new(
        handle: H,
        outcome: watch::Receiver<Option<Outcome>>,
        info: TrackInfo,
        url: String,
    ) -> Self {
        Self {
            handle,
            outcome,
            info,
            url,
            started: Instant::now(),
            confirmed: false,
            recorded: false,
            paused: false,
            position: Duration::ZERO,
        }
    }

    fn state(&self, error: Option<String>) -> State {
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
            ..State::default()
        }
    }

    fn entry(&self) -> HistoryEntry {
        HistoryEntry {
            url: self.url.clone(),
            title: self.info.title.clone(),
            duration_secs: self.info.duration.map(|duration| duration.as_secs()),
        }
    }

    fn stalled(&self) -> bool {
        !self.confirmed && self.started.elapsed() > START_TIMEOUT
    }

    fn observe(&mut self, info: &TrackState) -> Observed {
        match &info.playing {
            PlayMode::Errored(error) => return Observed::Done(Some(error.to_string())),
            mode if mode.is_done() => return Observed::Done(None),
            PlayMode::Play if self.paused || info.position.is_zero() => return Observed::Idle,
            PlayMode::Pause if !self.paused => return Observed::Idle,
            _ => {}
        }
        let position = Duration::from_secs(info.position.as_secs());
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

/// An in-flight connect-and-extract request. At most one exists at a time.
struct Job {
    task: JoinHandle<Result<Resolved>>,
    connected: Option<oneshot::Receiver<()>>,
    url: String,
}

enum JobEvent {
    /// Voice is ready (`true`), or the job ended without connecting (`false`).
    Connected(bool),
    Done(Result<Result<Resolved>, JoinError>),
}

async fn job_event(job: &mut Option<Job>) -> JobEvent {
    let Some(job) = job else {
        return pending().await;
    };
    if let Some(receiver) = job.connected.as_mut() {
        tokio::select! {
            biased;
            reached = receiver => {
                job.connected = None;
                return JobEvent::Connected(reached.is_ok());
            },
            result = &mut job.task => return JobEvent::Done(result),
        }
    }
    JobEvent::Done((&mut job.task).await)
}

async fn track_outcome<H>(track: &mut Option<Active<H>>) -> bool {
    match track {
        Some(current) => current.outcome.changed().await.is_ok(),
        None => pending().await,
    }
}

/// All playback, queue and volume state lives here. The GUI never mutates a copy.
pub(super) struct Controller<B: Backend> {
    backend: Arc<B>,
    health: watch::Receiver<Health>,
    states: watch::Sender<State>,
    persist: watch::Sender<Saved>,
    saved: Saved,
    queue: Queue,
    volume: u8,
    volume_seq: u64,
    notice: Option<String>,
    job: Option<Job>,
    track: Option<Active<B::Handle>>,
}

impl<B: Backend> Controller<B> {
    pub(super) fn new(
        backend: Arc<B>,
        saved: Saved,
        notice: Option<String>,
        states: watch::Sender<State>,
        persist: watch::Sender<Saved>,
    ) -> Self {
        Self {
            health: backend.health(),
            backend,
            states,
            persist,
            volume: saved.volume,
            saved,
            queue: Queue::default(),
            volume_seq: 0,
            notice,
            job: None,
            track: None,
        }
    }

    fn gateway(&self) -> bool {
        self.health.borrow().gateway
    }

    /// Adds the shared data (volume, queue, history) to a state about the track/phase.
    fn view(&self, mut state: State) -> State {
        state.volume = self.volume;
        state.volume_seq = self.volume_seq;
        state.queue = self.queue.snapshot();
        state.history.clone_from(&self.saved.history);
        state.notice.clone_from(&self.notice);
        state
    }

    fn emit(&self, state: State) {
        let state = self.view(state);
        if self.states.borrow().phase != state.phase {
            tracing::info!(state = ?state.phase, "Player state");
        }
        self.states.send_replace(state);
    }

    fn publish(&self, phase: Phase, error: Option<String>, available: bool) {
        self.emit(State {
            phase,
            error,
            available,
            ..State::default()
        });
    }

    /// Refreshes the shared data and always answers, so no GUI action is left waiting.
    fn sync(&self) {
        self.states
            .send_modify(|state| *state = self.view(std::mem::take(state)));
    }

    /// Shows a message without disturbing what is playing or being prepared.
    fn report(&self, message: String) {
        self.states.send_modify(|state| {
            state.error = Some(message);
            if state.phase == Phase::Ready {
                state.phase = Phase::Error;
            }
        });
    }

    fn persist(&self) {
        self.persist.send_replace(self.saved.clone());
    }

    async fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.task.abort();
            let _ = job.task.await;
        }
    }

    fn stop_track(&mut self) {
        if let Some(track) = self.track.take()
            && let Err(error) = track.handle.stop()
        {
            tracing::debug!(%error, "Track already finished");
        }
    }

    fn fail(&mut self, message: &str) {
        self.stop_track();
        self.publish(Phase::Error, Some(message.into()), true);
    }

    /// Starts `url`, cancelling whatever was being prepared or played. Only one request
    /// and one track can exist because everything runs through this single path.
    async fn begin(&mut self, url: String) {
        self.cancel_job().await;
        self.stop_track();
        self.backend.clear_error();
        self.publish(Phase::Connecting, None, true);
        let (connected_tx, connected) = oneshot::channel();
        let backend = self.backend.clone();
        let request = url.clone();
        self.job = Some(Job {
            task: tokio::spawn(async move { backend.prepare(request, connected_tx).await }),
            connected: Some(connected),
            url,
        });
    }

    /// Called exactly once per track that ended on its own: the next queued track, or Ready.
    async fn advance(&mut self) {
        match self.queue.pop_front() {
            Some(item) => self.begin(item.url).await,
            None => self.publish(Phase::Ready, None, true),
        }
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Stop => self.stop().await,
            Command::Pause => self.set_paused(true),
            Command::Resume => self.set_paused(false),
            Command::Play(value) => {
                if self.job.is_some() || !self.gateway() {
                    return;
                }
                match video_url(&value) {
                    Ok(url) => self.begin(url).await,
                    // A bad URL must not disturb the track that is still playing or paused.
                    Err(error) => self.report(error.to_string()),
                }
            }
            Command::Enqueue(value) => match video_url(&value) {
                Ok(url) => {
                    let title = self.saved.title_of(&url);
                    match self.queue.push(url, title) {
                        Ok(_) => self.sync(),
                        Err(Full) => {
                            self.report(format!("The queue is full ({QUEUE_LIMIT} tracks)."));
                        }
                    }
                }
                Err(error) => self.report(error.to_string()),
            },
            Command::Next => {
                if !self.gateway() {
                    return self.sync();
                }
                match self.queue.pop_front() {
                    Some(item) => self.begin(item.url).await,
                    None => self.sync(),
                }
            }
            Command::Dequeue(id) => {
                self.queue.remove(id);
                self.sync();
            }
            Command::Shift { id, up } => {
                self.queue.shift(id, up);
                self.sync();
            }
            Command::ClearQueue => {
                self.queue.clear();
                self.sync();
            }
            Command::ForgetHistory(url) => {
                if self.saved.remove(&url) {
                    self.persist();
                }
                self.sync();
            }
            Command::ClearHistory => {
                if !self.saved.history.is_empty() {
                    self.saved.history.clear();
                    self.persist();
                }
                self.sync();
            }
        }
    }

    /// Cancels pending work and the active track. The queue is kept and nothing advances.
    async fn stop(&mut self) {
        self.publish(Phase::Stopping, None, true);
        self.cancel_job().await;
        self.stop_track();
        self.backend.recover().await;
        let (connected, error) = {
            let health = self.health.borrow();
            (health.gateway, health.error())
        };
        let phase = if error.is_some() {
            Phase::Error
        } else {
            Phase::Ready
        };
        self.publish(phase, error, connected);
    }

    /// Applies Pause/Resume. The controller is the authority: it always answers with a state,
    /// so a stale or duplicate click can never leave the GUI waiting or showing the wrong button.
    fn set_paused(&mut self, paused: bool) {
        self.set_paused_with(paused, |handle| {
            if paused {
                handle.pause()
            } else {
                handle.resume()
            }
        });
    }

    fn set_paused_with(&mut self, paused: bool, send: impl FnOnce(&B::Handle) -> TrackResult<()>) {
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

    /// Applies the newest slider value to the live track, without restarting it.
    fn set_volume(&mut self, request: VolumeRequest) {
        let percent = request.percent.min(100);
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
    fn handle_outcome(&mut self, alive: bool) -> bool {
        let Some(current) = self.track.as_mut() else {
            return false;
        };
        if !alive {
            self.fail("Audio driver terminated unexpectedly. Try again.");
            return false;
        }
        let outcome = current.outcome.borrow_and_update().clone();
        match outcome {
            Some(Outcome::End) => {
                self.stop_track();
                true
            }
            Some(Outcome::Error(error)) => {
                self.fail(&error);
                false
            }
            None => false,
        }
    }

    /// Reads the track position and status through the existing handle; publishes at most
    /// once per displayed second or state change. Returns true when the track ended on its own.
    async fn poll(&mut self) -> bool {
        let Some(current) = self.track.as_ref() else {
            return false;
        };
        let reading = timeout(Duration::from_secs(2), current.handle.info()).await;
        self.apply(reading.ok())
    }

    fn apply(&mut self, reading: Option<TrackResult<TrackState>>) -> bool {
        let Some(current) = self.track.as_mut() else {
            return false;
        };
        let failure = match reading {
            Some(Ok(info)) => match current.observe(&info) {
                Observed::Changed => {
                    let state = current.state(None);
                    // History records a video once it is really playing, not when yt-dlp succeeds.
                    let entry = (!current.recorded).then(|| {
                        current.recorded = true;
                        current.entry()
                    });
                    if let Some(entry) = entry {
                        self.saved.record(entry);
                        self.persist();
                    }
                    self.emit(state);
                    None
                }
                Observed::Done(error) => {
                    self.stop_track();
                    let ended = error.is_none();
                    if let Some(error) = error {
                        self.publish(Phase::Error, Some(error), true);
                    }
                    return ended;
                }
                Observed::Idle => current
                    .stalled()
                    .then_some("Audio playback did not start. Try another video."),
            },
            Some(Err(error)) => {
                tracing::debug!(%error, "Track handle finished");
                // Allow terminal events to arrive; a dead driver cannot hold the UI forever.
                (current.started.elapsed() > START_TIMEOUT)
                    .then_some("Audio track terminated unexpectedly. Try again.")
            }
            None => Some("Audio driver stopped responding. Try again."),
        };
        if let Some(message) = failure {
            self.fail(message);
        }
        false
    }

    async fn finish_job(&mut self, result: Result<Result<Resolved>, JoinError>) {
        let Some(job) = self.job.take() else {
            return;
        };
        let resolved = result
            .context("Audio request task failed.")
            .and_then(|result| result);
        match resolved {
            Ok(resolved) => {
                let info = resolved.info.clone();
                match self
                    .backend
                    .play(resolved, volume_factor(self.volume))
                    .await
                {
                    Ok((handle, outcome)) => {
                        let current = Active::new(handle, outcome, info, job.url);
                        self.emit(current.state(None));
                        self.track = Some(current);
                    }
                    Err(error) => {
                        tracing::error!(error = %format!("{error:#}"), "Could not observe audio track");
                        self.publish(
                            Phase::Error,
                            Some("Could not start audio playback. Try again.".into()),
                            true,
                        );
                    }
                }
            }
            Err(error) => {
                tracing::error!(error = %format!("{error:#}"), "Audio request failed");
                self.backend.recover().await;
                let gateway = self.gateway();
                self.publish(Phase::Error, Some(media::user_error(&error)), gateway);
            }
        }
    }

    pub(super) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut volume: watch::Receiver<VolumeRequest>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut health = self.health.clone();
        // The only timer: drives status polling and the once-per-second position update.
        let mut tick = interval(Duration::from_millis(250));
        self.publish(Phase::Ready, None, true);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                command = commands.recv() => match command {
                    Some(command) => self.command(command).await,
                    None => break,
                },
                request = volume.changed() => {
                    if request.is_err() {
                        break;
                    }
                    let request = *volume.borrow_and_update();
                    self.set_volume(request);
                },
                changed = health.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let current = health.borrow_and_update().clone();
                    if let Some(error) = current.error() {
                        self.cancel_job().await;
                        self.stop_track();
                        self.publish(Phase::Error, Some(error), current.gateway);
                    } else if current.gateway
                        && self.states.borrow().phase == Phase::Error
                        && self.job.is_none()
                        && self.track.is_none()
                    {
                        self.publish(Phase::Ready, None, true);
                    }
                },
                event = job_event(&mut self.job) => match event {
                    JobEvent::Connected(reached) => {
                        if reached && self.states.borrow().phase == Phase::Connecting {
                            self.publish(Phase::Resolving, None, true);
                        }
                    }
                    JobEvent::Done(result) => self.finish_job(result).await,
                },
                alive = track_outcome(&mut self.track) => {
                    if self.handle_outcome(alive) {
                        self.advance().await;
                    }
                },
                _ = tick.tick(), if self.track.is_some() => {
                    if self.poll().await {
                        self.advance().await;
                    }
                },
            }
        }
        self.cancel_job().await;
        self.stop_track();
        self.backend.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
