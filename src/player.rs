use anyhow::{Context, Result};
use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler, TrackEvent,
    tracks::{PlayMode, TrackHandle, TrackResult, TrackState},
};
use std::{
    future::pending,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{interval, timeout},
};

use crate::{
    config::{Config, video_url},
    discord::Discord,
    media::{self, Resolved, TrackInfo},
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

#[derive(Clone, Debug)]
pub struct State {
    pub phase: Phase,
    pub error: Option<String>,
    pub available: bool,
    /// Metadata of the current track while it is playing or paused.
    pub track: Option<TrackInfo>,
    /// Playback position, truncated to whole seconds so it changes at most once per second.
    pub position: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            phase: Phase::Initializing,
            error: None,
            available: false,
            track: None,
            position: Duration::ZERO,
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
    Play(String),
    Pause,
    Resume,
    Stop,
}

fn publish_state(states: &watch::Sender<State>, state: State) {
    if states.borrow().phase != state.phase {
        tracing::info!(state = ?state.phase, "Player state");
    }
    states.send_replace(state);
}

fn publish(states: &watch::Sender<State>, phase: Phase, error: Option<String>, available: bool) {
    publish_state(
        states,
        State {
            phase,
            error,
            available,
            ..State::default()
        },
    );
}

#[derive(Clone, Debug)]
enum Outcome {
    End,
    Error(String),
}

struct TrackEvents(watch::Sender<Option<Outcome>>);

#[async_trait]
impl EventHandler for TrackEvents {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(tracks) = context {
            for (state, _) in *tracks {
                let outcome = match &state.playing {
                    PlayMode::Errored(error) => {
                        tracing::error!(%error, "Audio track failed");
                        Outcome::Error(
                            "Audio playback failed. Check media access and the logs.".into(),
                        )
                    }
                    _ => Outcome::End,
                };
                self.0.send_replace(Some(outcome));
            }
        }
        Some(Event::Cancel)
    }
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
struct Active {
    handle: TrackHandle,
    outcome: watch::Receiver<Option<Outcome>>,
    info: TrackInfo,
    started: Instant,
    /// Position has advanced at least once; Playing is not claimed before this.
    confirmed: bool,
    paused: bool,
    position: Duration,
}

impl Active {
    fn new(
        handle: TrackHandle,
        outcome: watch::Receiver<Option<Outcome>>,
        info: TrackInfo,
    ) -> Self {
        Self {
            handle,
            outcome,
            info,
            started: Instant::now(),
            confirmed: false,
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

async fn cancel_job(job: &mut Option<JoinHandle<Result<Resolved>>>) {
    if let Some(job) = job.take() {
        job.abort();
        let _ = job.await;
    }
}

fn stop_track(track: &mut Option<Active>) {
    if let Some(track) = track.take()
        && let Err(error) = track.handle.stop()
    {
        tracing::debug!(%error, "Track already finished");
    }
}

fn fail(track: &mut Option<Active>, states: &watch::Sender<State>, message: &str) {
    stop_track(track);
    publish(states, Phase::Error, Some(message.into()), true);
}

/// Applies Pause/Resume. The controller is the authority: it always answers with a state,
/// so a stale or duplicate click can never leave the GUI waiting or showing the wrong button.
fn set_paused(track: &mut Option<Active>, states: &watch::Sender<State>, paused: bool) {
    set_paused_with(track, states, paused, |handle| {
        if paused {
            handle.pause()
        } else {
            handle.play()
        }
    });
}

fn set_paused_with(
    track: &mut Option<Active>,
    states: &watch::Sender<State>,
    paused: bool,
    send: impl FnOnce(&TrackHandle) -> TrackResult<()>,
) {
    let Some(current) = track.as_mut() else {
        states.send_modify(|_| {});
        return;
    };
    if !current.confirmed || current.paused == paused {
        states.send_replace(current.state(None));
        return;
    }
    match send(&current.handle) {
        Ok(()) => {
            current.paused = paused;
            publish_state(states, current.state(None));
        }
        Err(error) => {
            tracing::warn!(%error, paused, "Track control failed");
            let action = if paused { "pause" } else { "resume" };
            fail(
                track,
                states,
                &format!("Could not {action} playback. The track is no longer active."),
            );
        }
    }
}

/// `alive` is false when the track's event senders disappeared without reporting an outcome.
fn handle_outcome(track: &mut Option<Active>, states: &watch::Sender<State>, alive: bool) {
    let Some(current) = track.as_mut() else {
        return;
    };
    if !alive {
        fail(
            track,
            states,
            "Audio driver terminated unexpectedly. Try again.",
        );
        return;
    }
    let outcome = current.outcome.borrow_and_update().clone();
    match outcome {
        Some(Outcome::End) => {
            stop_track(track);
            publish(states, Phase::Ready, None, true);
        }
        Some(Outcome::Error(error)) => fail(track, states, &error),
        None => {}
    }
}

/// Reads the track position and status through the existing handle; publishes at most
/// once per displayed second or state change.
async fn poll(track: &mut Option<Active>, states: &watch::Sender<State>) {
    let Some(current) = track.as_mut() else {
        return;
    };
    let failure = match timeout(Duration::from_secs(2), current.handle.get_info()).await {
        Ok(Ok(info)) => match current.observe(&info) {
            Observed::Changed => {
                publish_state(states, current.state(None));
                None
            }
            Observed::Done(error) => {
                stop_track(track);
                let phase = if error.is_some() {
                    Phase::Error
                } else {
                    Phase::Ready
                };
                publish(states, phase, error, true);
                None
            }
            Observed::Idle => current
                .stalled()
                .then_some("Audio playback did not start. Try another video."),
        },
        Ok(Err(error)) => {
            tracing::debug!(%error, "Track handle finished");
            // Allow terminal events to arrive; a dead driver cannot hold the UI forever.
            (current.started.elapsed() > START_TIMEOUT)
                .then_some("Audio track terminated unexpectedly. Try again.")
        }
        Err(_) => Some("Audio driver stopped responding. Try again."),
    };
    if let Some(message) = failure {
        fail(track, states, message);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Keep the biased select loop together so command priority, cancellation and track ownership remain explicit"
)]
pub async fn run(
    config: Result<Config>,
    mut commands: mpsc::Receiver<Command>,
    states: watch::Sender<State>,
    mut shutdown: watch::Receiver<bool>,
) {
    let startup = async {
        let config = config?;
        media::check_tools().await?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()
            .context("Could not create audio HTTP client.")?;
        Ok::<_, anyhow::Error>((Arc::new(Discord::start(config).await?), http))
    };
    let startup = tokio::select! {
        biased;
        _ = shutdown.changed() => return,
        result = startup => result,
    };
    let (discord, http) = match startup {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "Player initialization failed");
            publish(&states, Phase::Error, Some(error.to_string()), false);
            tokio::select! { _ = shutdown.changed() => {}, _ = commands.recv() => {} }
            return;
        }
    };
    let mut health = discord.health.clone();
    let mut job: Option<JoinHandle<Result<Resolved>>> = None;
    let mut track: Option<Active> = None;
    // The only timer: drives status polling and the once-per-second position update.
    let mut tick = interval(Duration::from_millis(250));
    publish(&states, Phase::Ready, None, true);

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            command = commands.recv() => {
                match command {
                    None => break,
                    Some(Command::Stop) => {
                        publish(&states, Phase::Stopping, None, true);
                        cancel_job(&mut job).await;
                        stop_track(&mut track);
                        discord.reset_incomplete_connection().await;
                        let connected = health.borrow().gateway;
                        let error = health.borrow().error();
                        publish(&states, if error.is_some() { Phase::Error } else { Phase::Ready }, error, connected);
                    },
                    Some(Command::Pause) => set_paused(&mut track, &states, true),
                    Some(Command::Resume) => set_paused(&mut track, &states, false),
                    Some(Command::Play(value)) => {
                        if job.is_some() || !health.borrow().gateway { continue; }
                        let url = match video_url(&value) {
                            Ok(url) => url,
                            Err(error) => {
                                // A bad URL must not disturb the track that is still playing or paused.
                                let message = Some(error.to_string());
                                match &track {
                                    Some(current) => publish_state(&states, current.state(message)),
                                    None => publish(&states, Phase::Error, message, true),
                                }
                                continue;
                            },
                        };
                        stop_track(&mut track);
                        discord.clear_voice_error();
                        publish(&states, Phase::Connecting, None, true);
                        let discord = discord.clone();
                        let http = http.clone();
                        let states = states.clone();
                        job = Some(tokio::spawn(async move {
                            discord.connect().await?;
                            publish(&states, Phase::Resolving, None, true);
                            media::resolve(http, url).await
                        }));
                    },
                }
            },
            changed = health.changed() => {
                if changed.is_err() { break; }
                let current = health.borrow_and_update().clone();
                if let Some(error) = current.error() {
                    cancel_job(&mut job).await;
                    stop_track(&mut track);
                    publish(&states, Phase::Error, Some(error), current.gateway);
                } else if current.gateway && states.borrow().phase == Phase::Error && job.is_none() && track.is_none() {
                    publish(&states, Phase::Ready, None, true);
                }
            },
            result = async {
                match &mut job { Some(job) => job.await, None => pending().await }
            } => {
                job = None;
                match result.context("Audio request task failed.").and_then(|result| result) {
                    Ok(Resolved { input, info }) => {
                        let call = discord.manager.get_or_insert(discord.guild);
                        let handle = call.lock().await.play_only_input(input);
                        let (sender, outcome) = watch::channel(None);
                        let registration = handle.add_event(Event::Track(TrackEvent::End), TrackEvents(sender.clone()))
                            .and_then(|()| handle.add_event(Event::Track(TrackEvent::Error), TrackEvents(sender)));
                        if let Err(error) = registration {
                            let _ = handle.stop();
                            tracing::error!(%error, "Could not observe audio track");
                            publish(&states, Phase::Error, Some("Could not start audio playback. Try again.".into()), true);
                        } else { track = Some(Active::new(handle, outcome, info)); }
                    },
                    Err(error) => {
                        tracing::error!(error = %format!("{error:#}"), "Audio request failed");
                        discord.reset_incomplete_connection().await;
                        publish(&states, Phase::Error, Some(media::user_error(&error)), health.borrow().gateway);
                    },
                }
            },
            alive = async {
                match &mut track { Some(current) => current.outcome.changed().await.is_ok(), None => pending().await }
            } => handle_outcome(&mut track, &states, alive),
            _ = tick.tick(), if track.is_some() => poll(&mut track, &states).await,
        }
    }
    cancel_job(&mut job).await;
    stop_track(&mut track);
    discord.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::{
        Active, Observed, Outcome, Phase, State, TrackEvents, cancel_job, handle_outcome,
        set_paused, set_paused_with, stop_track,
    };
    use crate::media::TrackInfo;
    use anyhow::{Context, Result, ensure};
    use songbird::{
        Driver, EventContext, EventHandler,
        tracks::{ControlError, PlayMode, TrackState},
    };
    use std::{sync::Arc, time::Duration};
    use tokio::{
        sync::{mpsc, watch},
        time::timeout,
    };

    fn state(phase: Phase, available: bool) -> State {
        State {
            phase,
            available,
            ..State::default()
        }
    }

    fn info(mode: PlayMode, seconds: f64) -> TrackState {
        TrackState {
            playing: mode,
            position: Duration::from_secs_f64(seconds),
            ..TrackState::default()
        }
    }

    /// A real Songbird handle on a detached driver, plus its event mailbox sender.
    fn track(driver: &mut Driver) -> (Active, watch::Sender<Option<Outcome>>) {
        let handle = driver.play_only_input(Vec::<u8>::new().into());
        let (sender, receiver) = watch::channel(None);
        let info = TrackInfo {
            title: Some("Song".into()),
            duration: Some(Duration::from_secs(225)),
        };
        (Active::new(handle, receiver, info), sender)
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

    #[tokio::test]
    async fn position_advances_once_per_second_and_freezes_while_paused() {
        let mut driver = Driver::new(songbird::Config::default());
        let (mut active, _sender) = track(&mut driver);
        // Not playing until the position actually moves.
        assert_eq!(active.observe(&info(PlayMode::Play, 0.0)), Observed::Idle);
        assert_eq!(active.state(None).phase, Phase::Resolving);
        assert_eq!(
            active.observe(&info(PlayMode::Play, 0.25)),
            Observed::Changed
        );
        assert_eq!(active.state(None).phase, Phase::Playing);
        // Sub-second polls are not published.
        assert_eq!(active.observe(&info(PlayMode::Play, 0.5)), Observed::Idle);
        assert_eq!(active.observe(&info(PlayMode::Play, 0.99)), Observed::Idle);
        assert_eq!(
            active.observe(&info(PlayMode::Play, 1.01)),
            Observed::Changed
        );
        assert_eq!(active.state(None).position, Duration::from_secs(1));
        // Paused: one correction at most, then silence; position never advances.
        active.paused = true;
        assert_eq!(active.observe(&info(PlayMode::Pause, 1.7)), Observed::Idle);
        assert_eq!(active.observe(&info(PlayMode::Pause, 1.7)), Observed::Idle);
        assert_eq!(active.state(None).phase, Phase::Paused);
        assert_eq!(active.state(None).position, Duration::from_secs(1));
        assert_eq!(
            active.observe(&info(PlayMode::Pause, 2.0)),
            Observed::Changed
        );
        // A Play report from before the pause took effect must not unpause the view.
        assert_eq!(active.observe(&info(PlayMode::Play, 2.5)), Observed::Idle);
        assert_eq!(active.state(None).phase, Phase::Paused);
        active.paused = false;
        assert_eq!(
            active.observe(&info(PlayMode::Play, 3.0)),
            Observed::Changed
        );
        assert_eq!(active.state(None).phase, Phase::Playing);
        assert_eq!(
            active.observe(&info(PlayMode::End, 9.0)),
            Observed::Done(None)
        );
        driver.stop();
    }

    #[tokio::test]
    async fn pause_and_resume_use_the_existing_track_handle() -> Result<()> {
        let mut driver = Driver::new(songbird::Config::default());
        let (active, _sender) = track(&mut driver);
        let (states, updates) = watch::channel(State::default());
        let mut slot = Some(active);
        // Before playback is confirmed neither command may claim a state change.
        set_paused(&mut slot, &states, true);
        ensure!(slot.as_ref().is_some_and(|track| !track.paused));
        if let Some(track) = slot.as_mut() {
            track.confirmed = true;
        }
        set_paused(&mut slot, &states, true);
        ensure!(updates.has_changed()?);
        assert_eq!(states.borrow().phase, Phase::Paused);
        assert_eq!(
            states
                .borrow()
                .track
                .as_ref()
                .and_then(|t| t.title.as_deref()),
            Some("Song")
        );
        // Duplicate Pause: still Paused, still answered so the GUI is released.
        set_paused(&mut slot, &states, true);
        assert_eq!(states.borrow().phase, Phase::Paused);
        set_paused(&mut slot, &states, false);
        assert_eq!(states.borrow().phase, Phase::Playing);
        // Duplicate Resume is idempotent.
        set_paused(&mut slot, &states, false);
        assert_eq!(states.borrow().phase, Phase::Playing);
        assert!(slot.is_some(), "Pause/Resume must keep the same track");
        driver.stop();
        Ok(())
    }

    #[tokio::test]
    async fn control_without_a_track_still_answers() {
        let (states, mut updates) = watch::channel(state(Phase::Ready, true));
        let mut slot = None;
        set_paused(&mut slot, &states, true);
        assert!(updates.has_changed().unwrap_or(false));
        assert_eq!(updates.borrow_and_update().phase, Phase::Ready);
    }

    #[tokio::test]
    async fn failed_control_command_reports_error_instead_of_a_false_state() {
        for paused in [true, false] {
            let mut driver = Driver::new(songbird::Config::default());
            let (mut active, _sender) = track(&mut driver);
            active.confirmed = true;
            active.paused = !paused;
            let (states, _updates) = watch::channel(active.state(None));
            let mut slot = Some(active);
            // The handle's channel is closed once its track is gone.
            set_paused_with(&mut slot, &states, paused, |_| Err(ControlError::Finished));
            assert!(slot.is_none(), "dead track must be released");
            let current = states.borrow().clone();
            assert_eq!(current.phase, Phase::Error);
            assert!(current.track.is_none());
            let action = if paused { "pause" } else { "resume" };
            assert!(current.error.is_some_and(|error| error.contains(action)));
            driver.stop();
        }
    }

    #[tokio::test]
    async fn stop_works_while_paused() {
        let mut driver = Driver::new(songbird::Config::default());
        let (mut active, _sender) = track(&mut driver);
        active.confirmed = true;
        active.paused = true;
        let mut slot = Some(active);
        stop_track(&mut slot);
        assert!(slot.is_none());
        driver.stop();
    }

    #[tokio::test]
    async fn replaced_track_cannot_end_its_successor() {
        let mut driver = Driver::new(songbird::Config::default());
        let (mut old, old_events) = track(&mut driver);
        old.confirmed = true;
        old.paused = true;
        let mut slot = Some(old);
        // Replacement while paused: old track is stopped and dropped, new one starts fresh.
        stop_track(&mut slot);
        let (new, _new_events) = track(&mut driver);
        slot = Some(new);
        assert!(slot.as_ref().is_some_and(|t| !t.paused && !t.confirmed));
        let (states, _updates) = watch::channel(state(Phase::Playing, true));
        // The old track's late End/Error lands in its own, now orphaned, mailbox.
        old_events.send_replace(Some(Outcome::End));
        old_events.send_replace(Some(Outcome::Error("stale".into())));
        handle_outcome(&mut slot, &states, true);
        assert!(slot.is_some());
        assert_eq!(states.borrow().phase, Phase::Playing);
        assert!(states.borrow().error.is_none());
        driver.stop();
    }

    #[tokio::test]
    async fn current_track_outcomes_end_or_fail_playback() {
        let mut driver = Driver::new(songbird::Config::default());
        let (states, _updates) = watch::channel(state(Phase::Playing, true));
        let (active, events) = track(&mut driver);
        let mut slot = Some(active);
        events.send_replace(Some(Outcome::End));
        handle_outcome(&mut slot, &states, true);
        assert!(slot.is_none());
        assert_eq!(states.borrow().phase, Phase::Ready);
        let (active, events) = track(&mut driver);
        slot = Some(active);
        events.send_replace(Some(Outcome::Error("boom".into())));
        handle_outcome(&mut slot, &states, true);
        assert!(slot.is_none());
        assert_eq!(states.borrow().phase, Phase::Error);
        assert_eq!(states.borrow().error.as_deref(), Some("boom"));
        let (active, _events) = track(&mut driver);
        slot = Some(active);
        handle_outcome(&mut slot, &states, false);
        assert!(slot.is_none());
        assert_eq!(states.borrow().phase, Phase::Error);
        driver.stop();
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn stop_cancels_pending_work_and_terminates_its_child() -> Result<()> {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut child = tokio::process::Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()?;
        let pid = child.id().context("test child has no process ID")?;
        let mut job = Some(tokio::spawn(async move {
            let _ = child.wait().await;
            let _ = sender.send(()).await;
            anyhow::bail!("unexpected completion")
        }));
        cancel_job(&mut job).await;
        assert!(job.is_none());
        assert_eq!(receiver.recv().await, None);
        timeout(Duration::from_secs(2), async {
            while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("cancelled request left its child running")?;
        Ok(())
    }

    #[tokio::test]
    async fn decode_error_is_reported_by_both_terminal_events() {
        let mut driver = songbird::Driver::new(songbird::Config::default());
        let handle = driver.play_only_input(Vec::<u8>::new().into());
        let (sender, receiver) = watch::channel(None);
        let handler = TrackEvents(sender);
        let state = songbird::tracks::TrackState {
            playing: PlayMode::Errored(songbird::tracks::PlayError::Decode(Arc::new(
                symphonia::core::errors::Error::DecodeError("invalid packet"),
            ))),
            ..Default::default()
        };
        let tracks = [(&state, &handle)];
        // Songbird emits Error and End for the same failure; neither may erase it.
        handler.act(&EventContext::Track(&tracks)).await;
        handler.act(&EventContext::Track(&tracks)).await;
        assert!(
            matches!(&*receiver.borrow(), Some(Outcome::Error(message)) if message.contains("playback failed"))
        );
        driver.stop();
    }
}
