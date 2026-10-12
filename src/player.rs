use anyhow::{Context, Result};
use songbird::tracks::{PlayMode, TrackResult, TrackState};
use std::{
    future::{Future, pending},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
    time::{interval, timeout},
};

use crate::{
    config::{Config, video_url},
    discord::{Discord, Health},
    live::Live,
    media::{self, Resolved, TrackInfo},
    queue::{Full, QUEUE_LIMIT, Queue, QueueItem},
    store::{self, HistoryEntry, Saved},
};

/// How long a track may take to start advancing before it is reported as failed.
const START_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Initializing,
    Ready,
    Connecting,
    Resolving,
    Playing,
    Paused,
    Stopping,
    Error,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Initializing => "Initializing…",
            Self::Ready => "Ready",
            Self::Connecting => "Connecting…",
            Self::Resolving => "Resolving audio…",
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopping => "Stopping…",
            Self::Error => "Error",
        }
    }
}

/// A snapshot of everything the window shows. The controller owns the truth; the GUI
/// only renders these and sends [`Command`]s and [`VolumeRequest`]s.
#[derive(Clone, Debug)]
pub struct State {
    pub phase: Phase,
    pub error: Option<String>,
    pub available: bool,
    /// Metadata of the current track while it is playing or paused.
    pub track: Option<TrackInfo>,
    /// Playback position, truncated to whole seconds so it changes at most once per second.
    pub position: Duration,
    /// Bot volume in percent, as actually applied.
    pub volume: u8,
    /// The newest [`VolumeRequest::seq`] the controller has handled, applied or not.
    pub volume_seq: u64,
    pub queue: Vec<QueueItem>,
    pub history: Vec<HistoryEntry>,
    /// A non-fatal remark from startup, such as a recovered history file.
    pub notice: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            phase: Phase::Initializing,
            error: None,
            available: false,
            track: None,
            position: Duration::ZERO,
            volume: store::DEFAULT_VOLUME,
            volume_seq: 0,
            queue: Vec::new(),
            history: Vec::new(),
            notice: None,
        }
    }
}

impl State {
    pub fn can_play(&self) -> bool {
        self.available
            && matches!(
                self.phase,
                Phase::Ready | Phase::Playing | Phase::Paused | Phase::Error
            )
    }
    pub fn can_stop(&self) -> bool {
        self.available
            && matches!(
                self.phase,
                Phase::Connecting
                    | Phase::Resolving
                    | Phase::Playing
                    | Phase::Paused
                    | Phase::Error
            )
    }
    /// Next starts the first queued track, also while another one is still being prepared.
    pub fn can_next(&self) -> bool {
        self.available
            && !self.queue.is_empty()
            && !matches!(self.phase, Phase::Initializing | Phase::Stopping)
    }
    /// Queueing only records a URL, so it works in every phase once Discord is up.
    pub fn can_enqueue(&self) -> bool {
        self.available && self.phase != Phase::Initializing
    }
    /// The track to show: only while audio is actually playing or paused.
    pub fn visible_track(&self) -> Option<&TrackInfo> {
        self.track
            .as_ref()
            .filter(|_| matches!(self.phase, Phase::Playing | Phase::Paused))
    }
    pub fn can_pause(&self) -> bool {
        self.available && self.phase == Phase::Playing
    }
    pub fn can_resume(&self) -> bool {
        self.available && self.phase == Phase::Paused
    }
}

pub enum Command {
    /// Plays now, replacing the current track. The pending queue is untouched.
    Play(String),
    Enqueue(String),
    Next,
    Pause,
    Resume,
    Stop,
    Dequeue(u64),
    Shift {
        id: u64,
        up: bool,
    },
    ClearQueue,
    ForgetHistory(String),
    ClearHistory,
}

/// The latest volume the slider wants. It travels over a `watch` channel, so a drag
/// produces at most one pending value instead of a stream of commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeRequest {
    pub percent: u8,
    /// Increases with every request; echoed in [`State::volume_seq`] once handled.
    pub seq: u64,
    /// The user let go (or pressed a key): remember this value across restarts.
    pub commit: bool,
}

impl Default for VolumeRequest {
    fn default() -> Self {
        Self {
            percent: store::DEFAULT_VOLUME,
            seq: 0,
            commit: false,
        }
    }
}

/// Songbird volume factor for a percentage: 0.0 to 1.0, never amplified.
pub fn volume_factor(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

/// Whole percent for a slider value; `None` for NaN or infinity.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to 0..=100 first"
)]
pub fn percent_from(value: f32) -> Option<u8> {
    value
        .is_finite()
        .then(|| value.round().clamp(0.0, 100.0) as u8)
}

/// How a track ended, as reported by its own event mailbox.
#[derive(Clone, Debug)]
pub enum Outcome {
    End,
    Error(String),
}

/// What the controller needs from a playing track.
pub trait Handle: Send + Sync + 'static {
    fn set_volume(&self, volume: f32) -> TrackResult<()>;
    fn pause(&self) -> TrackResult<()>;
    fn resume(&self) -> TrackResult<()>;
    fn stop(&self) -> TrackResult<()>;
    fn info(&self) -> impl Future<Output = TrackResult<TrackState>> + Send;
}

/// The Discord and yt-dlp side of the controller, replaceable in tests.
pub trait Backend: Send + Sync + 'static {
    type Handle: Handle;
    fn health(&self) -> watch::Receiver<Health>;
    fn clear_error(&self);
    /// Joins voice, then extracts the audio. `connected` fires once voice is ready.
    fn prepare(
        &self,
        url: String,
        connected: oneshot::Sender<()>,
    ) -> impl Future<Output = Result<Resolved>> + Send;
    /// Starts the track at `volume` and returns its handle and its own event mailbox.
    fn play(
        &self,
        resolved: Resolved,
        volume: f32,
    ) -> impl Future<Output = Result<(Self::Handle, watch::Receiver<Option<Outcome>>)>> + Send;
    fn recover(&self) -> impl Future<Output = ()> + Send;
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

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
struct Controller<B: Backend> {
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
    fn new(
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

    async fn run(
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

pub async fn run(
    config: Result<Config>,
    commands: mpsc::Receiver<Command>,
    volume: watch::Receiver<VolumeRequest>,
    states: watch::Sender<State>,
    mut shutdown: watch::Receiver<bool>,
) {
    let loaded = store::open();
    if let Some(warning) = &loaded.warning {
        tracing::warn!(%warning, "Saved state");
    }
    let startup = async {
        let config = config?;
        media::check_tools().await?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()
            .context("Could not create audio HTTP client.")?;
        Ok::<_, anyhow::Error>(Live::new(Arc::new(Discord::start(config).await?), http))
    };
    let startup = tokio::select! {
        biased;
        _ = shutdown.changed() => return,
        result = startup => result,
    };
    let backend = match startup {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "Player initialization failed");
            states.send_replace(State {
                phase: Phase::Error,
                error: Some(error.to_string()),
                volume: loaded.saved.volume,
                history: loaded.saved.history,
                notice: loaded.warning,
                ..State::default()
            });
            let mut commands = commands;
            tokio::select! { _ = shutdown.changed() => {}, _ = commands.recv() => {} }
            return;
        }
    };
    let (persist, snapshots) = watch::channel(loaded.saved.clone());
    let saver = loaded.path.map(|path| store::spawn_saver(path, snapshots));
    let controller = Controller::new(backend, loaded.saved, loaded.warning, states, persist);
    controller.run(commands, volume, shutdown).await;
    // The controller (and its snapshot sender) is gone; wait for the final write.
    if let Some(saver) = saver {
        let _ = saver.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, bail, ensure};
    use songbird::tracks::ControlError;
    use std::{
        collections::HashSet,
        sync::{
            Mutex, PoisonError,
            atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
        },
    };
    use tokio::time::sleep;

    const A: &str = "https://www.youtube.com/watch?v=aaaaaaaaaaa";
    const B: &str = "https://www.youtube.com/watch?v=bbbbbbbbbbb";
    const C: &str = "https://www.youtube.com/watch?v=ccccccccccc";

    // ---- fakes -----------------------------------------------------------------------

    /// A track whose position, end and failures the test controls. Songbird's own handle
    /// only answers while a real voice connection is mixing, so it cannot be driven offline.
    struct FakeTrack {
        title: String,
        start_volume: f32,
        volume: Mutex<f32>,
        mode: Mutex<(PlayMode, Duration)>,
        calls: Mutex<Vec<&'static str>>,
        outcome: watch::Sender<Option<Outcome>>,
        dead: AtomicBool,
        stopped: AtomicBool,
    }

    impl FakeTrack {
        fn end(&self) {
            self.outcome.send_replace(Some(Outcome::End));
        }
        fn advance(&self, seconds: u64) {
            *self.mode.lock().unwrap_or_else(PoisonError::into_inner) =
                (PlayMode::Play, Duration::from_secs(seconds));
        }
        fn calls(&self) -> Vec<&'static str> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
        fn volume(&self) -> f32 {
            *self.volume.lock().unwrap_or_else(PoisonError::into_inner)
        }
        fn record(&self, call: &'static str) -> TrackResult<()> {
            if self.dead.load(SeqCst) {
                return Err(ControlError::Finished);
            }
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct FakeHandle(Arc<FakeTrack>);

    impl Handle for FakeHandle {
        fn set_volume(&self, volume: f32) -> TrackResult<()> {
            self.0.record("volume")?;
            *self.0.volume.lock().unwrap_or_else(PoisonError::into_inner) = volume;
            Ok(())
        }
        fn pause(&self) -> TrackResult<()> {
            self.0.record("pause")
        }
        fn resume(&self) -> TrackResult<()> {
            self.0.record("resume")
        }
        fn stop(&self) -> TrackResult<()> {
            self.0.stopped.store(true, SeqCst);
            Ok(())
        }
        fn info(&self) -> impl Future<Output = TrackResult<TrackState>> + Send {
            let (playing, position) = self
                .0
                .mode
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            std::future::ready(Ok(TrackState {
                playing,
                position,
                ..TrackState::default()
            }))
        }
    }

    struct Fake {
        health: watch::Sender<Health>,
        requests: Mutex<Vec<String>>,
        hung: Mutex<HashSet<String>>,
        failing: Mutex<HashSet<String>>,
        running: AtomicUsize,
        max_running: AtomicUsize,
        cancelled: AtomicUsize,
        tracks: Mutex<Vec<Arc<FakeTrack>>>,
        shut_down: AtomicBool,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                health: watch::channel(Health {
                    gateway: true,
                    voice: true,
                    ..Health::default()
                })
                .0,
                requests: Mutex::default(),
                hung: Mutex::default(),
                failing: Mutex::default(),
                running: AtomicUsize::new(0),
                max_running: AtomicUsize::new(0),
                cancelled: AtomicUsize::new(0),
                tracks: Mutex::default(),
                shut_down: AtomicBool::new(false),
            })
        }
        fn hang(&self, url: &str) {
            self.hung
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(url.into());
        }
        fn fail(&self, url: &str) {
            self.failing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(url.into());
        }
        fn requests(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
        fn track(&self, index: usize) -> Result<Arc<FakeTrack>> {
            let tracks = self.tracks.lock().unwrap_or_else(PoisonError::into_inner);
            tracks
                .get(index)
                .cloned()
                .context("track was never started")
        }
        fn started(&self) -> usize {
            self.tracks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .len()
        }
    }

    /// Counts a request as cancelled when its future is dropped before it finished.
    struct Request<'a>(&'a Fake, bool);

    impl Drop for Request<'_> {
        fn drop(&mut self) {
            self.0.running.fetch_sub(1, SeqCst);
            if !self.1 {
                self.0.cancelled.fetch_add(1, SeqCst);
            }
        }
    }

    impl Backend for Fake {
        type Handle = FakeHandle;

        fn health(&self) -> watch::Receiver<Health> {
            self.health.subscribe()
        }
        fn clear_error(&self) {}
        async fn prepare(&self, url: String, connected: oneshot::Sender<()>) -> Result<Resolved> {
            self.requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(url.clone());
            let now = self.running.fetch_add(1, SeqCst) + 1;
            self.max_running.fetch_max(now, SeqCst);
            let mut request = Request(self, false);
            let _ = connected.send(());
            let hung = self
                .hung
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&url);
            if hung {
                pending::<()>().await;
            }
            sleep(Duration::from_millis(5)).await;
            request.1 = true;
            if self
                .failing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&url)
            {
                bail!("yt-dlp extraction failed: Sign in to confirm you’re not a bot");
            }
            Ok(Resolved {
                input: Vec::<u8>::new().into(),
                info: TrackInfo {
                    title: Some(url),
                    duration: Some(Duration::from_secs(60)),
                },
            })
        }
        fn play(
            &self,
            resolved: Resolved,
            volume: f32,
        ) -> impl Future<Output = Result<(FakeHandle, watch::Receiver<Option<Outcome>>)>> + Send
        {
            let (outcome, mailbox) = watch::channel(None);
            let track = Arc::new(FakeTrack {
                title: resolved.info.title.unwrap_or_default(),
                start_volume: volume,
                volume: Mutex::new(volume),
                mode: Mutex::new((PlayMode::Play, Duration::ZERO)),
                calls: Mutex::default(),
                outcome,
                dead: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
            });
            self.tracks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(track.clone());
            std::future::ready(Ok((FakeHandle(track), mailbox)))
        }
        async fn recover(&self) {}
        async fn shutdown(&self) {
            self.shut_down.store(true, SeqCst);
        }
    }

    /// A running controller plus the channels the GUI would hold.
    struct Rig {
        fake: Arc<Fake>,
        commands: mpsc::Sender<Command>,
        volume: watch::Sender<VolumeRequest>,
        states: watch::Receiver<State>,
        saves: watch::Receiver<Saved>,
        shutdown: watch::Sender<bool>,
        task: JoinHandle<()>,
        seq: u64,
    }

    impl Rig {
        fn start(saved: Saved) -> Self {
            let fake = Fake::new();
            let (commands, receiver) = mpsc::channel(16);
            let (volume, volume_rx) = watch::channel(VolumeRequest::default());
            let (states, states_rx) = watch::channel(State::default());
            let (to_disk, snapshots) = watch::channel(saved.clone());
            let (shutdown, shutdown_rx) = watch::channel(false);
            let controller = Controller::new(fake.clone(), saved, None, states, to_disk);
            Self {
                fake,
                commands,
                volume,
                states: states_rx,
                saves: snapshots,
                shutdown,
                task: tokio::spawn(controller.run(receiver, volume_rx, shutdown_rx)),
                seq: 0,
            }
        }
        async fn new() -> Result<Self> {
            let mut rig = Self::start(Saved::default());
            rig.until(|s| s.phase == Phase::Ready).await?;
            Ok(rig)
        }
        async fn send(&self, command: Command) -> Result<()> {
            self.commands.send(command).await?;
            Ok(())
        }
        async fn play(&self, url: &str) -> Result<()> {
            self.send(Command::Play(url.into())).await
        }
        async fn enqueue(&self, url: &str) -> Result<()> {
            self.send(Command::Enqueue(url.into())).await
        }
        fn set_volume(&mut self, percent: u8, commit: bool) {
            self.seq += 1;
            self.volume.send_replace(VolumeRequest {
                percent,
                seq: self.seq,
                commit,
            });
        }
        async fn until(&mut self, check: impl Fn(&State) -> bool) -> Result<State> {
            let state = timeout(Duration::from_secs(5), self.states.wait_for(check))
                .await
                .context("state was never reached")??;
            Ok(state.clone())
        }
        async fn started(&self, count: usize) -> Result<()> {
            eventually(|| self.fake.started() >= count).await
        }
        fn state(&self) -> State {
            self.states.borrow().clone()
        }
        fn queued(&self) -> Vec<String> {
            self.state()
                .queue
                .into_iter()
                .map(|item| item.url)
                .collect()
        }
        /// Long enough for several 250 ms ticks and any wrongly triggered work to show up.
        async fn settle(&self) {
            sleep(Duration::from_millis(400)).await;
        }
    }

    async fn eventually(check: impl Fn() -> bool + Send + Sync) -> Result<()> {
        timeout(Duration::from_secs(5), async {
            while !check() {
                sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .context("condition was never met")
    }

    fn state(phase: Phase, available: bool) -> State {
        State {
            phase,
            available,
            ..State::default()
        }
    }

    // ---- conversions and availability ------------------------------------------------

    #[test]
    fn volume_percentages_map_to_safe_factors() {
        assert!(volume_factor(0).abs() < f32::EPSILON);
        assert!((volume_factor(50) - 0.5).abs() < f32::EPSILON);
        assert!((volume_factor(100) - 1.0).abs() < f32::EPSILON);
        // Never amplified, whatever arrives.
        assert!((volume_factor(255) - 1.0).abs() < f32::EPSILON);
        for percent in 0..=u8::MAX {
            assert!((0.0..=1.0).contains(&volume_factor(percent)));
        }
    }

    #[test]
    fn slider_values_become_whole_percentages() {
        assert_eq!(percent_from(0.0), Some(0));
        assert_eq!(percent_from(49.5), Some(50));
        assert_eq!(percent_from(100.0), Some(100));
        assert_eq!(percent_from(-12.0), Some(0));
        assert_eq!(percent_from(1e9), Some(100));
        assert_eq!(percent_from(f32::NAN), None);
        assert_eq!(percent_from(f32::INFINITY), None);
        assert_eq!(percent_from(f32::NEG_INFINITY), None);
    }

    #[test]
    fn actions_follow_operational_state() {
        for phase in [
            Phase::Initializing,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Stopping,
        ] {
            assert!(!state(phase, true).can_play());
        }
        for phase in [Phase::Ready, Phase::Playing, Phase::Paused, Phase::Error] {
            assert!(state(phase, true).can_play());
            assert!(!state(phase, false).can_play());
        }
        assert!(!state(Phase::Ready, true).can_stop());
        assert!(state(Phase::Resolving, true).can_stop());
        assert!(!state(Phase::Stopping, true).can_stop());
    }

    #[test]
    fn pause_resume_and_stop_availability() {
        // Pause only while playing, Resume only while paused, Stop in both.
        for phase in [
            Phase::Initializing,
            Phase::Ready,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Paused,
            Phase::Stopping,
            Phase::Error,
        ] {
            assert!(!state(phase, true).can_pause(), "{phase:?}");
        }
        for phase in [
            Phase::Initializing,
            Phase::Ready,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Playing,
            Phase::Stopping,
            Phase::Error,
        ] {
            assert!(!state(phase, true).can_resume(), "{phase:?}");
        }
        assert!(state(Phase::Playing, true).can_pause());
        assert!(state(Phase::Paused, true).can_resume());
        assert!(state(Phase::Paused, true).can_stop());
        assert!(state(Phase::Playing, true).can_stop());
        assert!(!state(Phase::Playing, false).can_pause());
        assert!(!state(Phase::Paused, false).can_resume());
    }

    #[test]
    fn next_needs_a_queued_track_and_a_settled_player() {
        let mut value = state(Phase::Ready, true);
        assert!(!value.can_next(), "empty queue");
        value.queue = vec![QueueItem {
            id: 0,
            url: A.into(),
            title: None,
        }];
        for phase in [
            Phase::Ready,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Playing,
            Phase::Paused,
            Phase::Error,
        ] {
            value.phase = phase;
            assert!(value.can_next(), "{phase:?}");
        }
        for phase in [Phase::Initializing, Phase::Stopping] {
            value.phase = phase;
            assert!(!value.can_next(), "{phase:?}");
        }
        value.phase = Phase::Ready;
        value.available = false;
        assert!(!value.can_next() && !value.can_enqueue());
        assert!(state(Phase::Playing, true).can_enqueue());
        assert!(!state(Phase::Initializing, true).can_enqueue());
    }

    #[test]
    fn track_is_visible_only_while_playing_or_paused() {
        let mut value = state(Phase::Playing, true);
        value.track = Some(TrackInfo::default());
        assert!(value.visible_track().is_some());
        value.phase = Phase::Paused;
        assert!(value.visible_track().is_some());
        for phase in [
            Phase::Ready,
            Phase::Connecting,
            Phase::Stopping,
            Phase::Error,
        ] {
            value.phase = phase;
            assert!(value.visible_track().is_none(), "{phase:?}");
        }
    }

    // ---- the active track -------------------------------------------------------------

    fn active(track: &Arc<FakeTrack>) -> Active<FakeHandle> {
        let (_, mailbox) = watch::channel(None);
        Active::new(
            FakeHandle(track.clone()),
            mailbox,
            TrackInfo {
                title: Some("Song".into()),
                duration: Some(Duration::from_secs(225)),
            },
            A.into(),
        )
    }

    fn info(mode: PlayMode, seconds: f64) -> TrackState {
        TrackState {
            playing: mode,
            position: Duration::from_secs_f64(seconds),
            ..TrackState::default()
        }
    }

    async fn track_of(fake: &Arc<Fake>) -> Result<Arc<FakeTrack>> {
        fake.play(
            Resolved {
                input: Vec::<u8>::new().into(),
                info: TrackInfo::default(),
            },
            1.0,
        )
        .await?;
        fake.track(fake.started() - 1)
    }

    #[tokio::test]
    async fn position_advances_once_per_second_and_freezes_while_paused() -> Result<()> {
        let fake = Fake::new();
        let mut current = active(&track_of(&fake).await?);
        // Not playing until the position actually moves.
        assert_eq!(current.observe(&info(PlayMode::Play, 0.0)), Observed::Idle);
        assert_eq!(current.state(None).phase, Phase::Resolving);
        assert_eq!(
            current.observe(&info(PlayMode::Play, 0.25)),
            Observed::Changed
        );
        assert_eq!(current.state(None).phase, Phase::Playing);
        // Sub-second polls are not published.
        assert_eq!(current.observe(&info(PlayMode::Play, 0.5)), Observed::Idle);
        assert_eq!(current.observe(&info(PlayMode::Play, 0.99)), Observed::Idle);
        assert_eq!(
            current.observe(&info(PlayMode::Play, 1.01)),
            Observed::Changed
        );
        assert_eq!(current.state(None).position, Duration::from_secs(1));
        // Paused: one correction at most, then silence; position never advances.
        current.paused = true;
        assert_eq!(current.observe(&info(PlayMode::Pause, 1.7)), Observed::Idle);
        assert_eq!(current.observe(&info(PlayMode::Pause, 1.7)), Observed::Idle);
        assert_eq!(current.state(None).phase, Phase::Paused);
        assert_eq!(current.state(None).position, Duration::from_secs(1));
        assert_eq!(
            current.observe(&info(PlayMode::Pause, 2.0)),
            Observed::Changed
        );
        // A Play report from before the pause took effect must not unpause the view.
        assert_eq!(current.observe(&info(PlayMode::Play, 2.5)), Observed::Idle);
        assert_eq!(current.state(None).phase, Phase::Paused);
        current.paused = false;
        assert_eq!(
            current.observe(&info(PlayMode::Play, 3.0)),
            Observed::Changed
        );
        assert_eq!(current.state(None).phase, Phase::Playing);
        assert_eq!(
            current.observe(&info(PlayMode::End, 9.0)),
            Observed::Done(None)
        );
        Ok(())
    }

    fn solo(saved: Saved) -> (Controller<Fake>, Arc<Fake>, watch::Receiver<State>) {
        let fake = Fake::new();
        let (states, updates) = watch::channel(State::default());
        let (persist, _) = watch::channel(saved.clone());
        (
            Controller::new(fake.clone(), saved, None, states, persist),
            fake,
            updates,
        )
    }

    #[tokio::test]
    async fn pause_and_resume_use_the_existing_track_handle() -> Result<()> {
        let (mut controller, fake, updates) = solo(Saved::default());
        let track = track_of(&fake).await?;
        controller.track = Some(active(&track));
        // Before playback is confirmed neither command may claim a state change.
        controller.set_paused(true);
        ensure!(controller.track.as_ref().is_some_and(|t| !t.paused));
        ensure!(track.calls().is_empty());
        if let Some(current) = controller.track.as_mut() {
            current.confirmed = true;
        }
        controller.set_paused(true);
        ensure!(updates.has_changed()?);
        assert_eq!(updates.borrow().phase, Phase::Paused);
        assert_eq!(
            updates
                .borrow()
                .track
                .as_ref()
                .and_then(|t| t.title.as_deref()),
            Some("Song")
        );
        // Duplicate Pause: still Paused, still answered so the GUI is released.
        controller.set_paused(true);
        assert_eq!(updates.borrow().phase, Phase::Paused);
        controller.set_paused(false);
        assert_eq!(updates.borrow().phase, Phase::Playing);
        // Duplicate Resume is idempotent.
        controller.set_paused(false);
        assert_eq!(updates.borrow().phase, Phase::Playing);
        assert_eq!(track.calls(), ["pause", "resume"]);
        ensure!(
            controller.track.is_some(),
            "Pause/Resume must keep the track"
        );
        Ok(())
    }

    #[tokio::test]
    async fn control_without_a_track_still_answers() {
        let (mut controller, _fake, mut updates) = solo(Saved::default());
        controller.publish(Phase::Ready, None, true);
        updates.borrow_and_update();
        controller.set_paused(true);
        assert!(updates.has_changed().unwrap_or(false));
        assert_eq!(updates.borrow_and_update().phase, Phase::Ready);
    }

    #[tokio::test]
    async fn failed_control_command_reports_error_instead_of_a_false_state() -> Result<()> {
        for paused in [true, false] {
            let (mut controller, fake, updates) = solo(Saved::default());
            let mut current = active(&track_of(&fake).await?);
            current.confirmed = true;
            current.paused = !paused;
            controller.track = Some(current);
            // The handle's channel is closed once its track is gone.
            controller.set_paused_with(paused, |_| Err(ControlError::Finished));
            ensure!(controller.track.is_none(), "dead track must be released");
            let current = updates.borrow().clone();
            assert_eq!(current.phase, Phase::Error);
            assert!(current.track.is_none());
            let action = if paused { "pause" } else { "resume" };
            assert!(current.error.is_some_and(|error| error.contains(action)));
        }
        Ok(())
    }

    #[tokio::test]
    async fn stop_works_while_paused() -> Result<()> {
        let (mut controller, fake, _updates) = solo(Saved::default());
        let track = track_of(&fake).await?;
        let mut current = active(&track);
        current.confirmed = true;
        current.paused = true;
        controller.track = Some(current);
        controller.stop().await;
        ensure!(controller.track.is_none() && track.stopped.load(SeqCst));
        assert_eq!(controller.states.borrow().phase, Phase::Ready);
        Ok(())
    }

    #[tokio::test]
    async fn current_track_outcomes_end_or_fail_playback() -> Result<()> {
        for (outcome, ended, phase) in [
            (Some(Outcome::End), true, Phase::Initializing),
            (Some(Outcome::Error("boom".into())), false, Phase::Error),
            (None, false, Phase::Initializing),
        ] {
            let (mut controller, fake, updates) = solo(Saved::default());
            let (sender, mailbox) = watch::channel(None);
            let mut current = active(&track_of(&fake).await?);
            current.outcome = mailbox;
            controller.track = Some(current);
            sender.send_replace(outcome);
            assert_eq!(controller.handle_outcome(true), ended);
            assert_eq!(updates.borrow().phase, phase);
            assert_eq!(controller.track.is_none(), ended || phase == Phase::Error);
        }
        // Vanished event senders fail the track instead of hanging the UI.
        let (mut controller, fake, updates) = solo(Saved::default());
        controller.track = Some(active(&track_of(&fake).await?));
        assert!(!controller.handle_outcome(false));
        assert_eq!(updates.borrow().phase, Phase::Error);
        assert_eq!(
            updates.borrow().error.as_deref(),
            Some("Audio driver terminated unexpectedly. Try again.")
        );
        Ok(())
    }

    #[tokio::test]
    async fn history_records_only_when_playback_begins() -> Result<()> {
        let (mut controller, fake, updates) = solo(Saved::default());
        let track = track_of(&fake).await?;
        controller.track = Some(active(&track));
        // Extraction succeeded and the track exists, but nothing has played yet.
        assert!(!controller.apply(Some(Ok(info(PlayMode::Play, 0.0)))));
        assert_eq!(controller.saved.history.len(), 0);
        assert!(!controller.apply(Some(Ok(info(PlayMode::Play, 0.3)))));
        assert_eq!(controller.saved.history.len(), 1);
        assert_eq!(controller.saved.history[0].url, A);
        assert_eq!(controller.saved.history[0].title.as_deref(), Some("Song"));
        assert_eq!(controller.saved.history[0].duration_secs, Some(225));
        // Later ticks, pause and resume do not record again.
        assert!(!controller.apply(Some(Ok(info(PlayMode::Play, 1.5)))));
        assert_eq!(controller.saved.history.len(), 1);
        assert_eq!(updates.borrow().history.len(), 1);
        // A natural end is reported to the caller, who advances the queue.
        assert!(controller.apply(Some(Ok(info(PlayMode::End, 2.0)))));
        ensure!(controller.track.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_start_is_never_recorded() -> Result<()> {
        let (mut controller, fake, updates) = solo(Saved::default());
        controller.track = Some(active(&track_of(&fake).await?));
        let error = PlayMode::Errored(songbird::tracks::PlayError::Decode(Arc::new(
            symphonia::core::errors::Error::DecodeError("invalid packet"),
        )));
        assert!(!controller.apply(Some(Ok(info(error, 0.0)))));
        assert_eq!(controller.saved.history.len(), 0);
        assert_eq!(updates.borrow().phase, Phase::Error);
        Ok(())
    }

    // ---- queue behavior through the real controller loop ------------------------------

    #[tokio::test]
    async fn adding_while_idle_never_starts_playback() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        let state = rig.until(|s| s.queue.len() == 2).await?;
        assert_eq!(state.phase, Phase::Ready);
        rig.settle().await;
        assert!(rig.fake.requests().is_empty(), "nothing may be resolved");
        assert_eq!(rig.queued(), [A, B]);
        Ok(())
    }

    #[tokio::test]
    async fn queued_entries_reuse_titles_known_from_history() -> Result<()> {
        let mut saved = Saved::default();
        saved.record(crate::store::HistoryEntry {
            url: A.into(),
            title: Some("Known".into()),
            duration_secs: None,
        });
        let mut rig = Rig::start(saved);
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        let state = rig.until(|s| s.queue.len() == 2).await?;
        assert_eq!(state.queue[0].title.as_deref(), Some("Known"));
        assert_eq!(state.queue[1].title, None);
        // Queued URLs are canonicalized and never resolved.
        rig.enqueue("https://youtu.be/ccccccccccc?t=5").await?;
        let state = rig.until(|s| s.queue.len() == 3).await?;
        assert_eq!(state.queue[2].url, C);
        assert_eq!(rig.fake.requests().len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn adding_while_playing_or_paused_keeps_the_current_track() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        let track = rig.fake.track(0)?;
        track.advance(1);
        rig.until(|s| s.phase == Phase::Playing).await?;
        rig.enqueue(B).await?;
        rig.send(Command::Pause).await?;
        rig.until(|s| s.phase == Phase::Paused).await?;
        rig.enqueue(C).await?;
        let state = rig.until(|s| s.queue.len() == 2).await?;
        assert_eq!(state.phase, Phase::Paused);
        ensure!(!track.stopped.load(SeqCst));
        assert_eq!(rig.fake.requests(), [A]);
        assert_eq!(rig.queued(), [B, C]);
        Ok(())
    }

    #[tokio::test]
    async fn natural_end_starts_the_next_track_exactly_once() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.enqueue(C).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake.track(0)?.end();
        rig.started(2).await?;
        rig.until(|s| s.queue.len() == 1).await?;
        // The old track reports its end again; it must not skip the new one.
        rig.fake.track(0)?.end();
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A, B]);
        assert_eq!(rig.queued(), [C]);
        ensure!(!rig.fake.track(1)?.stopped.load(SeqCst));
        rig.fake.track(1)?.end();
        rig.started(3).await?;
        rig.fake.track(2)?.end();
        let state = rig.until(|s| s.phase == Phase::Ready).await?;
        assert!(state.queue.is_empty() && state.error.is_none());
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A, B, C]);
        Ok(())
    }

    #[tokio::test]
    async fn play_replaces_the_current_track_without_touching_the_queue() -> Result<()> {
        let rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        // The replaced track's late End must not advance the queue.
        rig.play(C).await?;
        rig.started(2).await?;
        let old = rig.fake.track(0)?;
        ensure!(old.stopped.load(SeqCst));
        old.end();
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A, C]);
        assert_eq!(rig.queued(), [B]);
        ensure!(!rig.fake.track(1)?.stopped.load(SeqCst));
        // Play while a request is still in flight is ignored; there is one active request.
        assert_eq!(rig.fake.max_running.load(SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn next_starts_the_first_queued_track_when_idle_or_playing() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.send(Command::Next).await?;
        rig.settle().await;
        assert!(
            rig.fake.requests().is_empty(),
            "empty queue: Next does nothing"
        );
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        rig.send(Command::Next).await?;
        rig.started(1).await?;
        assert_eq!(rig.fake.track(0)?.title, A);
        rig.until(|s| s.queue.len() == 1).await?;
        rig.send(Command::Next).await?;
        rig.started(2).await?;
        ensure!(
            rig.fake.track(0)?.stopped.load(SeqCst),
            "Next replaces the track"
        );
        ensure!(!rig.fake.track(1)?.stopped.load(SeqCst));
        rig.until(|s| s.queue.is_empty()).await?;
        // With nothing left, Next leaves the current track alone.
        rig.send(Command::Next).await?;
        rig.settle().await;
        ensure!(!rig.fake.track(1)?.stopped.load(SeqCst));
        assert_eq!(rig.fake.requests(), [A, B]);
        Ok(())
    }

    #[tokio::test]
    async fn next_cancels_an_extraction_in_progress_and_never_overlaps() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.fake.hang(A);
        rig.fake.hang(B);
        for url in [A, B, C] {
            rig.enqueue(url).await?;
        }
        rig.until(|s| s.queue.len() == 3).await?;
        for started in 1..=3 {
            rig.send(Command::Next).await?;
            eventually(|| rig.fake.requests().len() == started).await?;
        }
        rig.started(1).await?;
        rig.until(|s| s.queue.is_empty()).await?;
        assert_eq!(rig.fake.requests(), [A, B, C]);
        // A and B were cancelled while extracting, one at a time.
        eventually(|| rig.fake.cancelled.load(SeqCst) == 2).await?;
        assert_eq!(rig.fake.max_running.load(SeqCst), 1);
        assert_eq!(rig.fake.track(0)?.title, C);
        assert_eq!(rig.fake.started(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_burst_of_next_commands_starts_one_request_at_a_time() -> Result<()> {
        let mut rig = Rig::new().await?;
        for url in [A, B, C] {
            rig.enqueue(url).await?;
        }
        rig.until(|s| s.queue.len() == 3).await?;
        for _ in 0..3 {
            rig.send(Command::Next).await?;
        }
        rig.until(|s| s.queue.is_empty()).await?;
        rig.started(1).await?;
        rig.settle().await;
        assert_eq!(rig.fake.max_running.load(SeqCst), 1);
        assert_eq!(
            rig.fake.started(),
            1,
            "only the last request may become a track"
        );
        assert_eq!(rig.fake.track(0)?.title, C);
        Ok(())
    }

    #[tokio::test]
    async fn stop_keeps_the_queue_and_never_advances() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.send(Command::Stop).await?;
        rig.send(Command::Stop).await?;
        let state = rig.until(|s| s.phase == Phase::Ready).await?;
        assert_eq!(state.queue.len(), 1);
        ensure!(rig.fake.track(0)?.stopped.load(SeqCst));
        // The stopped track's own End event arrives late and is ignored.
        rig.fake.track(0)?.end();
        rig.send(Command::Stop).await?;
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A]);
        assert_eq!(rig.queued(), [B]);
        assert!(rig.state().available, "Discord stays connected");
        Ok(())
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_extraction() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.fake.hang(A);
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.until(|s| s.phase == Phase::Resolving).await?;
        rig.send(Command::Stop).await?;
        rig.until(|s| s.phase == Phase::Ready).await?;
        eventually(|| rig.fake.cancelled.load(SeqCst) == 1).await?;
        rig.settle().await;
        assert_eq!(rig.fake.started(), 0);
        assert_eq!(rig.queued(), [B]);
        Ok(())
    }

    #[tokio::test]
    async fn pause_and_resume_never_touch_the_queue() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        let track = rig.fake.track(0)?;
        track.advance(2);
        rig.until(|s| s.phase == Phase::Playing).await?;
        rig.send(Command::Pause).await?;
        rig.until(|s| s.phase == Phase::Paused).await?;
        rig.settle().await;
        assert_eq!(
            rig.fake.requests(),
            [A],
            "Pause must not start the next track"
        );
        assert_eq!(rig.queued(), [B]);
        rig.send(Command::Resume).await?;
        rig.until(|s| s.phase == Phase::Playing).await?;
        assert_eq!(track.calls(), ["pause", "resume"]);
        assert_eq!(rig.queued(), [B]);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_extraction_keeps_the_queue_and_does_not_retry() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.fake.fail(A);
        rig.enqueue(B).await?;
        rig.play(A).await?;
        let state = rig.until(|s| s.phase == Phase::Error).await?;
        assert!(state.error.is_some_and(|e| e.contains("restricted")));
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A], "no automatic retry or skip");
        assert_eq!(rig.queued(), [B]);
        // The user can move on manually.
        rig.send(Command::Next).await?;
        rig.started(1).await?;
        assert_eq!(rig.fake.track(0)?.title, B);
        Ok(())
    }

    #[tokio::test]
    async fn a_failing_queued_track_stops_the_chain() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.fake.fail(B);
        rig.enqueue(B).await?;
        rig.enqueue(C).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake.track(0)?.end();
        rig.until(|s| s.phase == Phase::Error).await?;
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A, B], "C must wait for the user");
        assert_eq!(rig.queued(), [C]);
        Ok(())
    }

    #[tokio::test]
    async fn a_track_error_does_not_advance_either() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake
            .track(0)?
            .outcome
            .send_replace(Some(Outcome::Error("Audio playback failed.".into())));
        let state = rig.until(|s| s.phase == Phase::Error).await?;
        assert_eq!(state.error.as_deref(), Some("Audio playback failed."));
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A]);
        assert_eq!(rig.queued(), [B]);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_urls_never_interrupt_playback_or_the_queue() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play("not a url").await?;
        let state = rig.until(|s| s.error.is_some()).await?;
        assert_eq!(state.phase, Phase::Error);
        rig.play(A).await?;
        rig.started(1).await?;
        rig.play("https://example.com/x").await?;
        rig.enqueue("https://youtube.com/playlist?list=1").await?;
        rig.enqueue("").await?;
        rig.until(|s| s.error.as_deref().is_some_and(|e| e.contains("YouTube")))
            .await?;
        rig.settle().await;
        ensure!(!rig.fake.track(0)?.stopped.load(SeqCst));
        assert_eq!(rig.fake.requests(), [A]);
        assert_eq!(rig.queued().len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn the_queue_holds_a_hundred_entries() -> Result<()> {
        let mut rig = Rig::new().await?;
        for _ in 0..QUEUE_LIMIT {
            rig.enqueue(A).await?;
        }
        rig.until(|s| s.queue.len() == QUEUE_LIMIT).await?;
        rig.enqueue(B).await?;
        let state = rig.until(|s| s.error.is_some()).await?;
        assert!(state.error.is_some_and(|e| e.contains("full")));
        assert_eq!(rig.state().queue.len(), QUEUE_LIMIT);
        ensure!(rig.state().queue.iter().all(|item| item.url == A));
        Ok(())
    }

    #[tokio::test]
    async fn queue_management_is_deterministic_and_leaves_playback_alone() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        for url in [A, B, C] {
            rig.enqueue(url).await?;
        }
        let state = rig.until(|s| s.queue.len() == 3).await?;
        let ids: Vec<u64> = state.queue.iter().map(|item| item.id).collect();
        rig.send(Command::Shift {
            id: ids[2],
            up: true,
        })
        .await?;
        rig.send(Command::Shift {
            id: ids[0],
            up: true,
        })
        .await?;
        rig.send(Command::Shift { id: 999, up: false }).await?;
        rig.until(|s| s.queue[1].id == ids[2]).await?;
        assert_eq!(rig.queued(), [A, C, B]);
        rig.send(Command::Shift {
            id: ids[0],
            up: false,
        })
        .await?;
        rig.until(|s| s.queue[1].id == ids[0]).await?;
        assert_eq!(rig.queued(), [C, A, B]);
        // Removing the same entry twice, or one that already started, changes nothing else.
        rig.send(Command::Dequeue(ids[0])).await?;
        rig.send(Command::Dequeue(ids[0])).await?;
        rig.until(|s| s.queue.len() == 2).await?;
        assert_eq!(rig.queued(), [C, B]);
        rig.send(Command::ClearQueue).await?;
        rig.send(Command::ClearQueue).await?;
        rig.until(|s| s.queue.is_empty()).await?;
        ensure!(
            !rig.fake.track(0)?.stopped.load(SeqCst),
            "Clear must not stop the track"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_replayed_history_entry_moves_to_the_top() -> Result<()> {
        let mut rig = Rig::new().await?;
        for (index, url) in [A, B].into_iter().enumerate() {
            rig.play(url).await?;
            rig.started(index + 1).await?;
            rig.fake.track(index)?.advance(1);
            rig.until(|s| s.history.len() == index + 1 && s.phase == Phase::Playing)
                .await?;
        }
        assert_eq!(rig.saves.borrow().history[0].url, B);
        // Playing A again from History reorders instead of duplicating.
        rig.play(A).await?;
        rig.started(3).await?;
        rig.fake.track(2)?.advance(1);
        let state = rig
            .until(|s| s.history[0].url == A && s.phase == Phase::Playing)
            .await?;
        let urls: Vec<_> = state.history.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, [A, B]);
        assert_eq!(rig.saves.borrow().history, state.history);
        Ok(())
    }

    #[tokio::test]
    async fn history_entries_can_be_forgotten_and_cleared() -> Result<()> {
        let mut saved = Saved::default();
        for url in [A, B, C] {
            saved.record(crate::store::HistoryEntry {
                url: url.into(),
                title: None,
                duration_secs: None,
            });
        }
        let mut rig = Rig::start(saved);
        rig.until(|s| s.history.len() == 3).await?;
        rig.send(Command::ForgetHistory(B.into())).await?;
        rig.until(|s| s.history.len() == 2).await?;
        assert_eq!(rig.saves.borrow().history.len(), 2);
        rig.send(Command::ClearHistory).await?;
        rig.until(|s| s.history.is_empty()).await?;
        assert_eq!(rig.saves.borrow().history.len(), 0);
        Ok(())
    }

    // ---- volume --------------------------------------------------------------------

    #[tokio::test]
    async fn volume_changes_the_live_track_and_is_saved_only_when_committed() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        let track = rig.fake.track(0)?;
        assert!(
            (track.start_volume - 1.0).abs() < f32::EPSILON,
            "default is full volume"
        );
        rig.set_volume(40, false);
        let state = rig.until(|s| s.volume == 40).await?;
        assert_eq!(state.volume_seq, 1);
        assert!((track.volume() - 0.4).abs() < f32::EPSILON);
        assert_eq!(rig.saves.borrow().volume, 100, "dragging does not save");
        ensure!(!track.stopped.load(SeqCst), "the stream is not restarted");
        rig.set_volume(35, true);
        rig.until(|s| s.volume == 35).await?;
        assert_eq!(rig.saves.borrow().volume, 35);
        assert_eq!(rig.fake.started(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn only_the_newest_drag_value_matters() -> Result<()> {
        let mut rig = Rig::new().await?;
        for percent in 0..=100 {
            rig.set_volume(percent, false);
        }
        let state = rig.until(|s| s.volume_seq == 101).await?;
        assert_eq!(state.volume, 100);
        Ok(())
    }

    #[tokio::test]
    async fn volume_survives_pause_resume_next_and_replacement() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.play(A).await?;
        rig.started(1).await?;
        let first = rig.fake.track(0)?;
        first.advance(1);
        rig.until(|s| s.phase == Phase::Playing).await?;
        rig.send(Command::Pause).await?;
        rig.until(|s| s.phase == Phase::Paused).await?;
        // Adjustable while paused.
        rig.set_volume(25, true);
        rig.until(|s| s.volume == 25).await?;
        assert!((first.volume() - 0.25).abs() < f32::EPSILON);
        rig.send(Command::Resume).await?;
        rig.until(|s| s.phase == Phase::Playing).await?;
        assert!((first.volume() - 0.25).abs() < f32::EPSILON);
        rig.send(Command::Next).await?;
        rig.started(2).await?;
        assert!((rig.fake.track(1)?.start_volume - 0.25).abs() < f32::EPSILON);
        rig.play(C).await?;
        rig.started(3).await?;
        assert!((rig.fake.track(2)?.start_volume - 0.25).abs() < f32::EPSILON);
        Ok(())
    }

    #[tokio::test]
    async fn the_saved_volume_applies_to_the_first_track() -> Result<()> {
        let mut rig = Rig::start(Saved {
            volume: 35,
            ..Saved::default()
        });
        let state = rig.until(|s| s.phase == Phase::Ready).await?;
        assert_eq!(state.volume, 35);
        rig.play(A).await?;
        rig.started(1).await?;
        assert!((rig.fake.track(0)?.start_volume - 0.35).abs() < f32::EPSILON);
        Ok(())
    }

    #[tokio::test]
    async fn volume_without_a_track_is_a_preference_for_the_next_one() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.set_volume(60, true);
        rig.until(|s| s.volume == 60).await?;
        assert_eq!(rig.saves.borrow().volume, 60);
        rig.play(A).await?;
        rig.started(1).await?;
        assert!((rig.fake.track(0)?.start_volume - 0.6).abs() < f32::EPSILON);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_volume_change_is_reported_not_claimed() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake.track(0)?.dead.store(true, SeqCst);
        rig.set_volume(10, true);
        let state = rig.until(|s| s.phase == Phase::Error).await?;
        assert_eq!(state.volume, 100, "the old volume stays");
        assert_eq!(state.volume_seq, 1, "the request is answered");
        assert!(state.error.is_some_and(|e| e.contains("volume")));
        assert_eq!(rig.saves.borrow().volume, 100);
        Ok(())
    }

    // ---- lifecycle --------------------------------------------------------------------

    #[tokio::test]
    async fn shutdown_cancels_work_and_releases_discord() -> Result<()> {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake.hang(B);
        rig.enqueue(B).await?;
        rig.send(Command::Next).await?;
        eventually(|| rig.fake.requests().len() == 2).await?;
        rig.shutdown.send_replace(true);
        timeout(Duration::from_secs(5), &mut rig.task).await??;
        ensure!(rig.fake.track(0)?.stopped.load(SeqCst));
        eventually(|| rig.fake.cancelled.load(SeqCst) == 1).await?;
        ensure!(rig.fake.shut_down.load(SeqCst));
        Ok(())
    }
}
