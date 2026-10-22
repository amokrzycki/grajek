use super::*;
use crate::{player::percent_from, queue::QueueItem};
use anyhow::{Result, bail, ensure};
use songbird::tracks::ControlError;
use std::future::Future;
use std::{
    collections::{HashMap, HashSet},
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
    info_hung: AtomicBool,
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
    async fn info(&self) -> TrackResult<TrackState> {
        if self.0.info_hung.load(SeqCst) {
            pending::<()>().await;
        }
        let (playing, position) = self
            .0
            .mode
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(TrackState {
            playing,
            position,
            ..TrackState::default()
        })
    }
}

struct Fake {
    health: watch::Sender<Health>,
    requests: Mutex<Vec<String>>,
    hung: Mutex<HashSet<String>>,
    failing: Mutex<HashSet<String>>,
    play_failing: Mutex<HashSet<String>>,
    disconnect_hung: AtomicBool,
    disconnect_failing: AtomicBool,
    playlist_reply: Mutex<Option<oneshot::Receiver<Result<crate::playlist::Playlist>>>>,
    playlist_calls: AtomicUsize,
    title_requests: Mutex<Vec<String>>,
    title_replies: Mutex<HashMap<String, oneshot::Receiver<Result<Option<String>>>>>,
    running: AtomicUsize,
    max_running: AtomicUsize,
    cancelled: AtomicUsize,
    tracks: Mutex<Vec<Arc<FakeTrack>>>,
    shut_down: AtomicBool,
    connects: AtomicUsize,
    disconnects: AtomicUsize,
    connect_mode: AtomicUsize,
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
            play_failing: Mutex::default(),
            disconnect_hung: AtomicBool::new(false),
            disconnect_failing: AtomicBool::new(false),
            playlist_reply: Mutex::default(),
            playlist_calls: AtomicUsize::new(0),
            title_requests: Mutex::default(),
            title_replies: Mutex::default(),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            tracks: Mutex::default(),
            shut_down: AtomicBool::new(false),
            connects: AtomicUsize::new(0),
            disconnects: AtomicUsize::new(0),
            connect_mode: AtomicUsize::new(0),
        })
    }
    fn import_reply(&self) -> oneshot::Sender<Result<crate::playlist::Playlist>> {
        let (sender, receiver) = oneshot::channel();
        *self
            .playlist_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(receiver);
        sender
    }
    fn title_reply(&self, url: &str) -> oneshot::Sender<Result<Option<String>>> {
        let (sender, receiver) = oneshot::channel();
        self.title_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(url.into(), receiver);
        sender
    }
    fn title_requests(&self) -> Vec<String> {
        self.title_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        if !self.health.borrow().voice {
            self.connect().await?;
        }
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
    ) -> impl Future<Output = Result<(FakeHandle, watch::Receiver<Option<Outcome>>)>> + Send {
        if resolved.info.title.as_ref().is_some_and(|title| {
            self.play_failing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(title)
        }) {
            return std::future::ready(Err(anyhow::anyhow!("Driver rejected track")));
        }
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
            info_hung: AtomicBool::new(false),
        });
        self.tracks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(track.clone());
        std::future::ready(Ok((FakeHandle(track), mailbox)))
    }
    async fn title(&self, url: String) -> Result<Option<String>> {
        self.title_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.clone());
        let reply = self
            .title_replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&url);
        match reply {
            Some(reply) => reply.await?,
            None => Ok(None),
        }
    }
    async fn playlist(&self, _url: String) -> Result<crate::playlist::Playlist> {
        self.playlist_calls.fetch_add(1, SeqCst);
        let reply = self
            .playlist_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        reply.context("not configured")?.await?
    }
    async fn connect(&self) -> Result<()> {
        self.connects.fetch_add(1, SeqCst);
        match self.connect_mode.load(SeqCst) {
            1 => {
                self.health.send_modify(|health| {
                    health.voice_retry = true;
                    health.voice_error = Some("Transient voice failure".into());
                });
                bail!("Transient voice failure");
            }
            2 => pending::<()>().await,
            _ => {}
        }
        self.health.send_modify(|health| {
            health.voice = true;
            health.voice_error = None;
            health.voice_retry = false;
        });
        Ok(())
    }
    async fn disconnect(&self) -> Result<()> {
        self.disconnects.fetch_add(1, SeqCst);
        if self.disconnect_hung.load(SeqCst) {
            pending::<()>().await;
        }
        if self.disconnect_failing.load(SeqCst) {
            bail!("Discord teardown failed");
        }
        self.health.send_modify(|health| {
            health.voice = false;
            health.voice_error = None;
            health.voice_retry = false;
        });
        Ok(())
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
        title_unavailable: false,
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
    controller.publish(Phase::Ready, None);
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
        assert_eq!(controller.track.is_none(), phase == Phase::Error);
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
    controller.natural_end().await;
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
async fn a_healthy_update_does_not_erase_a_media_error() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.fake.fail(A);
    rig.enqueue(B).await?;
    rig.play(A).await?;
    let failed = rig.until(|s| s.phase == Phase::Error).await?;
    rig.fake.health.send_modify(|health| health.voice = true);
    rig.settle().await;
    assert_eq!(rig.state().phase, Phase::Error);
    assert_eq!(rig.state().error, failed.error);
    assert_eq!(rig.queued(), [B]);
    Ok(())
}

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

// ---- v0.2.1 reliability -----------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn queued_media_failure_can_continue_but_explicit_play_cannot() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.send(Command::ContinueOnError(true)).await?;
    rig.until(|s| s.preferences.continue_on_error).await?;
    assert!(rig.saves.borrow().continue_on_error);
    rig.fake.fail(A);
    rig.enqueue(A).await?;
    rig.enqueue(B).await?;
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.until(|s| s.queue.is_empty()).await?;
    assert_eq!(rig.fake.requests(), [A, B]);
    assert!(rig.state().error.is_some_and(|e| e.contains("Skipped")));
    assert_eq!(rig.state().history.len(), 0);
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.enqueue(C).await?;
    rig.play(A).await?;
    rig.until(|s| s.phase == Phase::Error).await?;
    tokio::time::advance(Duration::from_secs(10)).await;
    rig.settle().await;
    assert_eq!(rig.fake.requests(), [A, B, A]);
    assert_eq!(rig.queued(), [C]);
    assert_eq!(rig.fake.max_running.load(SeqCst), 1);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn three_failures_stop_recovery_without_discarding_the_rest() -> Result<()> {
    let mut rig = Rig::start(Saved {
        continue_on_error: true,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.fail(A);
    for url in [A, A, A, B] {
        rig.enqueue(url).await?;
    }
    rig.send(Command::Next).await?;
    let state = rig
        .until(|s| {
            s.error
                .as_deref()
                .is_some_and(|e| e.contains("3 consecutive"))
        })
        .await?;
    assert_eq!(state.phase, Phase::Error);
    assert_eq!(rig.queued(), [B]);
    tokio::time::advance(Duration::from_secs(30)).await;
    rig.settle().await;
    assert_eq!(rig.fake.requests(), [A, A, A]);
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    assert_eq!(rig.fake.track(0)?.title, B);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stop_and_next_cancel_a_scheduled_queue_skip() -> Result<()> {
    for stop in [true, false] {
        let mut rig = Rig::start(Saved {
            continue_on_error: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.fake.fail(A);
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        rig.enqueue(C).await?;
        rig.send(Command::Next).await?;
        rig.until(|s| s.phase == Phase::Error).await?;
        rig.send(if stop { Command::Stop } else { Command::Next })
            .await?;
        if stop {
            rig.until(|s| s.phase == Phase::Ready).await?;
        } else {
            rig.started(1).await?;
        }
        tokio::time::advance(Duration::from_secs(5)).await;
        rig.settle().await;
        assert_eq!(rig.queued(), if stop { vec![B, C] } else { vec![C] });
        assert_eq!(rig.fake.requests(), if stop { vec![A] } else { vec![A, B] });
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn only_real_playback_resets_the_consecutive_failure_budget() -> Result<()> {
    let mut rig = Rig::start(Saved {
        continue_on_error: true,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.fail(A);
    for url in [A, B, A, A, A, C] {
        rig.enqueue(url).await?;
    }
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    // B has a handle but did not advance: its failure is the second, not the first.
    rig.fake
        .track(0)?
        .outcome
        .send_replace(Some(Outcome::Error("decode".into())));
    rig.until(|s| {
        s.error
            .as_deref()
            .is_some_and(|e| e.contains("3 consecutive"))
    })
    .await?;
    assert_eq!(rig.queued(), [A, A, C]);
    rig.play(B).await?;
    rig.started(2).await?;
    rig.fake.track(1)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.track(1)?.end();
    rig.started(3).await?;
    assert_eq!(rig.fake.requests(), [A, B, A, B, A, A, C]);
    assert_eq!(rig.queued().len(), 0);
    Ok(())
}

fn lose_voice(fake: &Fake, retry: bool) {
    fake.health.send_modify(|health| {
        health.voice = false;
        health.voice_retry = retry;
        health.voice_error = Some("Voice connection lost".into());
    });
}

#[tokio::test(start_paused = true)]
async fn voice_recovery_restores_connectivity_without_restarting_audio() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.play(A).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    lose_voice(&rig.fake, true);
    rig.until(|s| s.phase == Phase::Error).await?;
    // Duplicate health updates cannot reset the delay or spawn another job.
    lose_voice(&rig.fake, true);
    let state = rig
        .until(|s| s.phase == Phase::Ready && s.voice_connected)
        .await?;
    assert!(state.error.is_some_and(|e| e.contains("Press Next")));
    assert_eq!(rig.fake.connects.load(SeqCst), 1);
    assert_eq!(rig.fake.requests(), [A]);
    assert_eq!(rig.fake.started(), 1);
    assert_eq!(rig.queued(), [B]);
    assert!(rig.fake.track(0)?.stopped.load(SeqCst));
    rig.fake.track(0)?.end();
    rig.settle().await;
    assert_eq!(rig.queued(), [B]);
    rig.send(Command::Next).await?;
    rig.started(2).await?;
    assert_eq!(rig.fake.track(1)?.title, B);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn exhausted_voice_recovery_preserves_queue_and_manual_play_still_works() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.fake.connect_mode.store(1, SeqCst);
    lose_voice(&rig.fake, true);
    rig.until(|s| s.error.as_deref().is_some_and(|e| e.contains("2 attempts")))
        .await?;
    assert_eq!(rig.fake.connects.load(SeqCst), 2);
    assert_eq!(rig.queued(), [B]);
    lose_voice(&rig.fake, true);
    tokio::time::advance(Duration::from_secs(60)).await;
    rig.settle().await;
    assert_eq!(rig.fake.connects.load(SeqCst), 2);
    rig.fake.connect_mode.store(0, SeqCst);
    rig.play(A).await?;
    rig.started(1).await?;
    assert_eq!(rig.queued(), [B]);
    assert_eq!(rig.state().phase, Phase::Resolving);
    Ok(())
}

#[tokio::test]
async fn a_rejoin_result_classifies_failure_before_the_driver_callback() -> Result<()> {
    use songbird::error::{ConnectionError, JoinError};
    use std::io::ErrorKind;

    for (error, terminal_error, retry) in [
        (
            JoinError::Driver(ConnectionError::Io(ErrorKind::ConnectionReset.into())),
            None,
            true,
        ),
        (JoinError::Driver(ConnectionError::TimedOut), None, true),
        (JoinError::TimedOut, None, false),
        (JoinError::NoSender, None, false),
        (
            JoinError::Driver(ConnectionError::CryptoModeInvalid),
            None,
            false,
        ),
        (
            JoinError::Driver(ConnectionError::Io(ErrorKind::ConnectionReset.into())),
            Some("Kicked; check permissions"),
            false,
        ),
    ] {
        let (mut controller, fake, updates) = solo(Saved::default());
        fake.health.send_modify(|health| {
            health.voice = false;
            health.voice_error = terminal_error.map(str::to_owned);
        });
        controller.command(Command::Enqueue(B.into())).await;
        controller.job = Some(Job {
            task: tokio::spawn(async move {
                Err(anyhow::Error::new(error).context("Cannot join Discord voice."))
            }),
            connected: None,
            kind: JobKind::Rejoin(1),
            voice_ready: false,
        });
        let JobEvent::Done(result) = job_event(&mut controller.job).await else {
            bail!("A rejoin must not report media preparation");
        };
        // No driver callback has set voice_retry; the typed result is already definitive.
        controller.finish_job(result).await;
        assert_eq!(
            matches!(controller.scheduled, Some((_, Scheduled::Rejoin(2)))),
            retry
        );
        assert_eq!(updates.borrow().queue[0].url, B);
        assert!(updates.borrow().track.is_none());
        assert_eq!(fake.started(), 0);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stop_next_and_shutdown_cancel_voice_recovery() -> Result<()> {
    for action in [0, 1, 2] {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.fake.connect_mode.store(2, SeqCst);
        lose_voice(&rig.fake, true);
        eventually(|| rig.fake.connects.load(SeqCst) == 1).await?;
        rig.fake.connect_mode.store(0, SeqCst);
        match action {
            0 => {
                rig.send(Command::Stop).await?;
                rig.until(|s| s.phase == Phase::Error || s.phase == Phase::Ready)
                    .await?;
                rig.settle().await;
                assert_eq!(rig.queued(), [B]);
                assert_eq!(rig.fake.started(), 0);
            }
            1 => {
                rig.send(Command::Next).await?;
                rig.started(1).await?;
                assert_eq!(rig.fake.track(0)?.title, B);
                assert_eq!(rig.fake.requests(), [B]);
            }
            _ => {
                rig.shutdown.send_replace(true);
                timeout(Duration::from_secs(5), &mut rig.task).await??;
                assert!(rig.fake.shut_down.load(SeqCst));
            }
        }
        tokio::time::advance(Duration::from_secs(60)).await;
        rig.settle().await;
        assert_eq!(
            rig.fake.connects.load(SeqCst),
            if action == 1 { 2 } else { 1 }
        );
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn terminal_voice_and_gateway_failures_never_skip_or_rejoin() -> Result<()> {
    for gateway in [true, false] {
        let mut rig = Rig::start(Saved {
            continue_on_error: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        rig.fake.hang(A);
        rig.send(Command::Next).await?;
        rig.until(|s| s.phase == Phase::Resolving).await?;
        rig.fake.health.send_modify(|health| {
            health.voice = false;
            health.gateway = gateway;
            health.voice_retry = false;
            if gateway {
                health.voice_error = Some("Kicked or unauthorized".into());
            } else {
                health.gateway_error = Some("Terminal gateway failure".into());
            }
        });
        rig.until(|s| s.phase == Phase::Error).await?;
        tokio::time::advance(Duration::from_secs(60)).await;
        rig.settle().await;
        assert_eq!(rig.fake.requests(), [A]);
        assert_eq!(rig.fake.connects.load(SeqCst), 0);
        assert_eq!(rig.queued(), [B]);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn idle_disconnect_keeps_data_and_play_and_next_reconnect() -> Result<()> {
    for next in [true, false] {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        tokio::time::advance(Duration::from_secs(599)).await;
        rig.settle().await;
        assert_eq!(rig.fake.disconnects.load(SeqCst), 0);
        tokio::time::advance(Duration::from_secs(1)).await;
        let state = rig.until(|s| !s.voice_connected).await?;
        assert_eq!(state.phase, Phase::Ready);
        assert!(state.error.is_none() && state.history.is_empty());
        assert_eq!(rig.queued(), [B]);
        tokio::time::advance(IDLE_TIMEOUT * 2).await;
        rig.settle().await;
        assert_eq!(rig.fake.disconnects.load(SeqCst), 1);
        rig.send(if next {
            Command::Next
        } else {
            Command::Play(A.into())
        })
        .await?;
        rig.started(1).await?;
        assert_eq!(rig.fake.connects.load(SeqCst), 1);
        assert_eq!(rig.fake.track(0)?.title, if next { B } else { A });
        assert_eq!(rig.queued().len(), usize::from(!next));
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn playing_paused_and_extracting_are_active_until_stop() -> Result<()> {
    for phase in [Phase::Playing, Phase::Paused, Phase::Resolving] {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        if phase == Phase::Resolving {
            rig.fake.hang(A);
        }
        rig.play(A).await?;
        if phase != Phase::Resolving {
            rig.started(1).await?;
            rig.fake.track(0)?.advance(1);
            rig.until(|s| s.phase == Phase::Playing).await?;
            if phase == Phase::Paused {
                rig.send(Command::Pause).await?;
            }
        }
        rig.until(|s| s.phase == phase).await?;
        tokio::time::advance(IDLE_TIMEOUT * 2).await;
        rig.settle().await;
        assert_eq!(rig.fake.disconnects.load(SeqCst), 0);
        rig.send(Command::Stop).await?;
        rig.until(|s| s.phase == Phase::Ready).await?;
        tokio::time::advance(IDLE_TIMEOUT).await;
        rig.until(|s| !s.voice_connected).await?;
        assert_eq!(rig.fake.disconnects.load(SeqCst), 1);
        assert_eq!(rig.queued(), [B]);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stop_is_responsive_while_track_info_is_unresponsive() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.play(A).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.info_hung.store(true, SeqCst);
    tokio::time::advance(Duration::from_millis(300)).await;
    tokio::task::yield_now().await;
    let before = Instant::now();
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert!(before.elapsed() < Duration::from_millis(100));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn play_cancels_a_voice_rejoin_in_progress() -> Result<()> {
    let rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.fake.connect_mode.store(2, SeqCst);
    lose_voice(&rig.fake, true);
    eventually(|| rig.fake.connects.load(SeqCst) == 1).await?;
    rig.fake.connect_mode.store(0, SeqCst);
    rig.play(A).await?;
    rig.started(1).await?;
    assert_eq!(rig.fake.requests(), [A]);
    assert_eq!(rig.queued(), [B]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_terminal_event_cancels_pending_voice_recovery() -> Result<()> {
    for running in [false, true] {
        let mut rig = Rig::new().await?;
        rig.enqueue(B).await?;
        rig.fake.connect_mode.store(2, SeqCst);
        lose_voice(&rig.fake, true);
        rig.until(|s| s.phase == Phase::Error).await?;
        if running {
            eventually(|| rig.fake.connects.load(SeqCst) == 1).await?;
        }
        rig.fake.health.send_modify(|health| {
            health.voice_retry = false;
            health.voice_error = Some("Kicked; check permissions".into());
        });
        rig.until(|s| s.error.as_deref() == Some("Kicked; check permissions"))
            .await?;
        tokio::time::advance(Duration::from_secs(60)).await;
        rig.settle().await;
        assert_eq!(rig.fake.connects.load(SeqCst), usize::from(running));
        assert_eq!(rig.queued(), [B]);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn play_during_idle_leave_cancels_the_leave_and_starts_normally() -> Result<()> {
    let rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.fake.disconnect_hung.store(true, SeqCst);
    tokio::time::advance(IDLE_TIMEOUT).await;
    eventually(|| rig.fake.disconnects.load(SeqCst) == 1).await?;
    assert!(rig.state().voice_connected);
    rig.play(A).await?;
    rig.started(1).await?;
    assert_eq!(rig.fake.requests(), [A]);
    assert_eq!(rig.queued(), [B]);
    assert_eq!(
        rig.fake.connects.load(SeqCst),
        0,
        "cancelled teardown kept voice connected"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_rejected_track_start_obeys_the_queue_recovery_preference() -> Result<()> {
    for enabled in [false, true] {
        let mut rig = Rig::start(Saved {
            continue_on_error: enabled,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.fake
            .play_failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(A.into());
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        rig.send(Command::Next).await?;
        rig.until(|s| s.phase == Phase::Error).await?;
        if enabled {
            rig.started(1).await?;
            assert_eq!(rig.fake.track(0)?.title, B);
            assert_eq!(rig.fake.requests(), [A, B]);
            assert_eq!(rig.queued().len(), 0);
        } else {
            tokio::time::advance(Duration::from_secs(5)).await;
            rig.settle().await;
            assert_eq!(rig.fake.started(), 0);
            assert_eq!(rig.queued(), [B]);
        }
        assert_eq!(rig.state().history.len(), 0);
    }
    Ok(())
}

// v0.3.0: observable pending-only storage and completion-driven repeat.
#[tokio::test(start_paused = true)]
async fn remembering_queue_tracks_edits_consumption_disable_and_restore() -> Result<()> {
    let mut rig = Rig::new().await?;
    for url in [A, B, A] {
        rig.enqueue(url).await?;
    }
    rig.until(|s| s.queue.len() == 3).await?;
    assert_eq!(rig.saves.borrow().queue.len(), 0);
    rig.send(Command::RememberQueue(true)).await?;
    let state = rig.until(|s| s.preferences.remember_queue).await?;
    let ids: HashSet<_> = state.queue.iter().map(|q| q.id).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        rig.saves
            .borrow()
            .queue
            .iter()
            .map(|q| q.url.as_str())
            .collect::<Vec<_>>(),
        [A, B, A]
    );
    rig.send(Command::Shift {
        id: state.queue[1].id,
        up: true,
    })
    .await?;
    rig.until(|s| s.queue[0].url == B).await?;
    rig.send(Command::Dequeue(state.queue[0].id)).await?;
    rig.until(|s| s.queue.len() == 2).await?;
    rig.enqueue(C).await?;
    rig.until(|s| s.queue.len() == 3).await?;
    let saved = rig.saves.borrow().clone();
    let mut restored = Rig::start(saved);
    let loaded = restored.until(|s| s.phase == Phase::Ready).await?;
    assert_eq!(restored.queued(), [B, A, C]);
    assert_eq!(
        loaded
            .queue
            .iter()
            .map(|q| q.id)
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    assert_eq!(restored.fake.requests().len(), 0);
    restored.send(Command::Next).await?;
    restored.started(1).await?;
    assert_eq!(
        restored
            .saves
            .borrow()
            .queue
            .iter()
            .map(|q| q.url.as_str())
            .collect::<Vec<_>>(),
        [A, C]
    );
    restored.send(Command::RememberQueue(false)).await?;
    restored.until(|s| !s.preferences.remember_queue).await?;
    assert_eq!(restored.saves.borrow().queue.len(), 0);
    assert_eq!(restored.queued(), [A, C]);
    restored.send(Command::RememberQueue(true)).await?;
    restored.until(|s| s.preferences.remember_queue).await?;
    restored.send(Command::ClearQueue).await?;
    restored.until(|s| s.queue.is_empty()).await?;
    restored.shutdown.send_replace(true);
    restored.task.await?;
    assert_eq!(restored.saves.borrow().queue.len(), 0);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_track_resolves_fresh_and_next_stop_and_stale_events_bypass_it() -> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: super::super::RepeatMode::Track,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(B).await?;
    rig.play(A).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.track(0)?.end();
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [A, A]);
    assert_eq!(rig.queued(), [B]);
    rig.fake.track(0)?.end();
    rig.settle().await;
    assert_eq!(rig.fake.started(), 2);
    rig.send(Command::Next).await?;
    rig.started(3).await?;
    assert_eq!(rig.fake.requests(), [A, A, B]);
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.track(2)?.end();
    rig.settle().await;
    assert_eq!(rig.fake.started(), 3);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_queue_cycles_fifo_and_singleton_without_growing() -> Result<()> {
    for urls in [vec![A], vec![A, B, C], vec![A, A, B]] {
        let mut rig = Rig::start(Saved {
            repeat: super::super::RepeatMode::Queue,
            remember_queue: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        for url in &urls {
            rig.enqueue(url).await?;
        }
        rig.send(Command::Next).await?;
        for n in 0..7 {
            rig.started(n + 1).await?;
            assert_eq!(rig.fake.track(n)?.title, urls[n % urls.len()]);
            rig.fake.track(n)?.advance(1);
            rig.until(|s| s.phase == Phase::Playing).await?;
            assert_eq!(rig.queued().len(), urls.len() - 1);
            assert_eq!(rig.saves.borrow().queue.len(), urls.len() - 1);
            rig.fake.track(n)?.end();
        }
        rig.started(8).await?;
        assert_eq!(rig.fake.max_running.load(SeqCst), 1);
        rig.send(Command::Stop).await?;
        rig.until(|s| s.phase == Phase::Ready).await?;
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_queue_respects_capacity_edits_direct_play_and_mode_changes() -> Result<()> {
    use super::super::RepeatMode;
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        remember_queue: true,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    for _ in 0..QUEUE_LIMIT {
        rig.enqueue(B).await?;
    }
    rig.until(|s| s.queue.len() == QUEUE_LIMIT).await?;
    rig.fake.track(0)?.end();
    rig.started(2).await?;
    let queued = rig
        .until(|s| s.phase == Phase::Resolving && s.queue.len() == QUEUE_LIMIT)
        .await?;
    assert_eq!(queued.queue.last().map(|q| q.url.as_str()), Some(A));
    assert_eq!(rig.saves.borrow().queue.len(), QUEUE_LIMIT);
    // Manual Next does not recycle B, even if it was confirmed.
    rig.fake.track(1)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.send(Command::Next).await?;
    rig.started(3).await?;
    assert_eq!(rig.queued().len(), QUEUE_LIMIT - 1);
    // Direct Play URL replaces the track without rewriting pending entries or joining its cycle.
    rig.fake.track(2)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let pending = rig.queued();
    rig.play(C).await?;
    rig.started(4).await?;
    assert_eq!(rig.queued(), pending);
    rig.fake.track(3)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.track(3)?.end();
    rig.started(5).await?;
    assert!(!rig.queued().iter().any(|q| q == C));
    // The mode in force at completion applies to the current queued track.
    rig.send(Command::Repeat(RepeatMode::Track)).await?;
    rig.until(|s| s.preferences.repeat == RepeatMode::Track)
        .await?;
    assert_eq!(rig.saves.borrow().repeat, RepeatMode::Track);
    rig.fake.track(4)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let pending = rig.queued();
    rig.fake.track(4)?.end();
    rig.started(6).await?;
    assert_eq!(rig.queued(), pending);
    rig.send(Command::Repeat(RepeatMode::Off)).await?;
    rig.until(|s| s.preferences.repeat == RepeatMode::Off)
        .await?;
    rig.fake.track(5)?.end();
    rig.started(7).await?;
    assert_eq!(rig.queued().len(), pending.len() - 1);
    rig.send(Command::ClearQueue).await?;
    rig.until(|s| s.queue.is_empty()).await?;
    rig.fake.track(6)?.end();
    rig.until(|s| s.phase == Phase::Ready).await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_never_recycles_failed_or_unconfirmed_tracks_and_keeps_failure_budget() -> Result<()>
{
    use super::super::RepeatMode;
    for repeat in [RepeatMode::Track, RepeatMode::Queue] {
        for failure in ["extract", "startup", "early-end", "stopped", "error"] {
            let mut rig = Rig::start(Saved {
                repeat,
                remember_queue: true,
                continue_on_error: true,
                ..Saved::default()
            });
            rig.until(|s| s.phase == Phase::Ready).await?;
            if failure == "extract" {
                rig.fake.fail(A);
            }
            if failure == "startup" {
                rig.fake
                    .play_failing
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(A.into());
            }
            for url in [A, A, A, B] {
                rig.enqueue(url).await?;
            }
            rig.send(Command::Next).await?;
            if !matches!(failure, "extract" | "startup") {
                for index in 0..3 {
                    rig.started(index + 1).await?;
                    let track = rig.fake.track(index)?;
                    match failure {
                        "early-end" => track.end(),
                        "stopped" => {
                            *track.mode.lock().unwrap_or_else(PoisonError::into_inner) =
                                (PlayMode::Stop, Duration::ZERO);
                        }
                        _ => {
                            track
                                .outcome
                                .send_replace(Some(Outcome::Error("decode failure".into())));
                        }
                    }
                }
            }
            rig.until(|s| {
                s.error
                    .as_deref()
                    .is_some_and(|e| e.contains("3 consecutive"))
            })
            .await?;
            tokio::time::advance(Duration::from_secs(30)).await;
            rig.settle().await;
            assert_eq!(rig.fake.requests(), [A, A, A], "{repeat:?}: {failure}");
            assert_eq!(rig.queued(), [B]);
            assert_eq!(rig.saves.borrow().queue.len(), 1);
            assert_eq!(rig.state().history.len(), 0);
        }
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_stop_cancels_fresh_extraction_and_never_restores_consumed_entry() -> Result<()> {
    use super::super::RepeatMode;
    for repeat in [RepeatMode::Track, RepeatMode::Queue] {
        let mut rig = Rig::start(Saved {
            repeat,
            remember_queue: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        rig.send(Command::Next).await?;
        rig.started(1).await?;
        rig.fake.track(0)?.advance(1);
        rig.until(|s| s.phase == Phase::Playing).await?;
        rig.fake
            .hung
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend([A.into(), B.into()]);
        rig.fake.track(0)?.end();
        eventually(|| rig.fake.requests().len() == 2).await?;
        rig.send(Command::Stop).await?;
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.fake.track(0)?.end();
        rig.settle().await;
        assert_eq!(rig.fake.requests().len(), 2);
        assert_eq!(rig.fake.cancelled.load(SeqCst), 1);
        let saved = rig.saves.borrow().clone();
        assert_eq!(
            saved.queue[0].url,
            if repeat == RepeatMode::Track { B } else { A }
        );
        rig.shutdown.send_replace(true);
        rig.task.await?;
        assert_eq!(*rig.saves.borrow(), saved);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn queue_repeat_direct_singleton_and_clear_have_explicit_completion_semantics() -> Result<()>
{
    use super::super::RepeatMode;
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    // A direct URL never becomes part of Repeat Queue, even with no pending entries.
    rig.play(A).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.track(0)?.end();
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert_eq!(rig.fake.requests(), [A]);
    rig.enqueue(B).await?;
    rig.enqueue(C).await?;
    rig.send(Command::Next).await?;
    rig.started(2).await?;
    rig.fake.track(1)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.send(Command::ClearQueue).await?;
    rig.until(|s| s.queue.is_empty()).await?;
    // Clear removes pending entries; the current successful queued track remains a singleton cycle.
    rig.fake.track(1)?.end();
    rig.started(3).await?;
    assert_eq!(rig.fake.requests(), [A, B, B]);
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert_eq!(rig.queued().len(), 0);
    Ok(())
}

// ---- queue metadata and explicit leave --------------------------------------------

#[tokio::test]
async fn fresh_queue_metadata_is_async_shared_and_persisted_without_playback() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.health.send_modify(|health| health.voice = false);
    let reply = rig.fake.title_reply(A);
    rig.enqueue("https://youtu.be/aaaaaaaaaaa?t=10").await?;
    rig.enqueue(A).await?;
    let pending = rig.until(|s| s.queue.len() == 2).await?;
    assert!(
        pending
            .queue
            .iter()
            .all(|item| item.label() == "Loading title…")
    );
    let ids: Vec<_> = pending.queue.iter().map(|item| item.id).collect();
    eventually(|| rig.fake.title_requests() == [A]).await?;
    reply
        .send(Ok(Some("  Żółć Привет 🎵  ".into())))
        .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
    let state = rig
        .until(|s| s.queue.iter().all(|item| item.title.is_some()))
        .await?;
    assert_eq!(
        state.queue.iter().map(|item| item.id).collect::<Vec<_>>(),
        ids
    );
    assert!(
        state
            .queue
            .iter()
            .all(|item| item.label() == "Żółć Привет 🎵")
    );
    assert_eq!(rig.fake.requests(), Vec::<String>::new());
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    assert_eq!(rig.fake.started(), 0);
    let saved: Saved = serde_json::from_str(&serde_json::to_string(&*rig.saves.borrow())?)?;
    let mut restored = Rig::start(saved);
    restored.until(|s| s.phase == Phase::Ready).await?;
    assert_eq!(restored.state().queue[0].label(), "Żółć Привет 🎵");
    assert_eq!(restored.fake.title_requests(), Vec::<String>::new());
    rig.enqueue(A).await?;
    rig.until(|s| s.queue.len() == 3).await?;
    assert_eq!(rig.fake.title_requests(), [A]);
    Ok(())
}

#[tokio::test]
async fn history_and_restored_queue_titles_avoid_lookup() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        history: vec![HistoryEntry {
            url: A.into(),
            title: Some("Known history".into()),
            duration_secs: None,
        }],
        queue: vec![SavedQueueItem {
            url: B.into(),
            title: Some("Known queue".into()),
        }],
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 3).await?;
    assert_eq!(rig.state().queue[1].label(), "Known history");
    assert_eq!(rig.state().queue[2].label(), "Known queue");
    assert_eq!(rig.fake.title_requests(), Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn failed_or_missing_metadata_keeps_queue_and_playback_usable() -> Result<()> {
    for result in [
        Ok(None),
        Err(anyhow::anyhow!("private video: raw diagnostics")),
    ] {
        let mut rig = Rig::new().await?;
        let reply = rig.fake.title_reply(A);
        rig.enqueue(A).await?;
        eventually(|| !rig.fake.title_requests().is_empty()).await?;
        reply
            .send(result)
            .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
        let state = rig
            .until(|s| s.queue.first().is_some_and(|item| item.title_unavailable))
            .await?;
        assert_eq!(state.queue[0].label(), "Title unavailable");
        assert!(state.error.is_none());
        rig.send(Command::Next).await?;
        rig.started(1).await?;
        assert_eq!(rig.fake.track(0)?.title, A);
    }
    Ok(())
}

#[tokio::test]
async fn metadata_targets_survive_reordering_and_cancel_on_removal_or_clear() -> Result<()> {
    for clear in [false, true] {
        let mut rig = Rig::new().await?;
        let mut old_reply = rig.fake.title_reply(A);
        let reply_b = rig.fake.title_reply(B);
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        let state = rig.until(|s| s.queue.len() == 2).await?;
        let first = state.queue[0].id;
        rig.send(Command::Shift {
            id: first,
            up: false,
        })
        .await?;
        rig.until(|s| s.queue[0].url == B).await?;
        assert_eq!(rig.fake.title_requests(), [A], "one lookup at a time");
        rig.send(if clear {
            Command::ClearQueue
        } else {
            Command::Dequeue(first)
        })
        .await?;
        timeout(Duration::from_secs(5), old_reply.closed()).await?;
        assert!(old_reply.send(Ok(Some("Stale title".into()))).is_err());
        if clear {
            rig.until(|s| s.queue.is_empty()).await?;
            rig.enqueue(B).await?;
        }
        eventually(|| rig.fake.title_requests() == [A, B]).await?;
        reply_b
            .send(Ok(Some("Correct B".into())))
            .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
        rig.until(|s| {
            s.queue
                .first()
                .is_some_and(|item| item.label() == "Correct B")
        })
        .await?;
        let reply_a = rig.fake.title_reply(A);
        rig.enqueue(A).await?;
        eventually(|| rig.fake.title_requests() == [A, B, A]).await?;
        reply_a
            .send(Ok(Some("New A".into())))
            .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
        let state = rig
            .until(|s| s.queue.len() == 2 && s.queue[1].title.is_some())
            .await?;
        assert_ne!(state.queue[1].id, first);
        assert_eq!(state.queue[1].label(), "New A");
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn metadata_timeout_and_shutdown_cancel_the_independent_job() -> Result<()> {
    let mut rig = Rig::new().await?;
    let mut reply = rig.fake.title_reply(A);
    rig.enqueue(A).await?;
    eventually(|| !rig.fake.title_requests().is_empty()).await?;
    tokio::time::advance(TITLE_TIMEOUT).await;
    rig.until(|s| s.queue[0].title_unavailable).await?;
    assert!(reply.is_closed());
    let mut reply_b = rig.fake.title_reply(B);
    rig.enqueue(B).await?;
    eventually(|| rig.fake.title_requests() == [A, B]).await?;
    rig.shutdown.send_replace(true);
    timeout(Duration::from_secs(5), &mut rig.task).await??;
    assert!(reply_b.is_closed());
    // closed() also proves the sender cannot deliver into an orphaned worker.
    reply.closed().await;
    reply_b.closed().await;
    Ok(())
}

#[tokio::test]
async fn full_duplicate_queue_shares_one_lookup_and_rejects_extra_work() -> Result<()> {
    let mut rig = Rig::new().await?;
    let reply = rig.fake.title_reply(A);
    for _ in 0..QUEUE_LIMIT {
        rig.enqueue(A).await?;
    }
    rig.until(|s| s.queue.len() == QUEUE_LIMIT).await?;
    rig.enqueue(B).await?;
    rig.until(|s| {
        s.error
            .as_deref()
            .is_some_and(|error| error.contains("full"))
    })
    .await?;
    assert_eq!(rig.fake.title_requests(), [A]);
    reply
        .send(Ok(Some("Shared".into())))
        .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
    rig.until(|s| s.queue.iter().all(|item| item.label() == "Shared"))
        .await?;
    assert_eq!(rig.state().queue.len(), QUEUE_LIMIT);
    assert_eq!(rig.fake.title_requests(), [A]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn leave_voice_stops_active_or_pending_audio_and_explicit_actions_reconnect() -> Result<()> {
    for phase in [
        Phase::Playing,
        Phase::Paused,
        Phase::Resolving,
        Phase::Connecting,
    ] {
        let mut rig = Rig::start(Saved {
            volume: 37,
            repeat: RepeatMode::Queue,
            continue_on_error: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.enqueue(B).await?;
        if phase == Phase::Connecting {
            rig.fake.health.send_modify(|health| health.voice = false);
            rig.fake.connect_mode.store(2, SeqCst);
        } else if phase == Phase::Resolving {
            rig.fake.hang(A);
        }
        rig.play(A).await?;
        if matches!(phase, Phase::Playing | Phase::Paused) {
            rig.started(1).await?;
            rig.fake.track(0)?.advance(1);
            rig.until(|s| s.phase == Phase::Playing).await?;
            if phase == Phase::Paused {
                rig.send(Command::Pause).await?;
            }
        }
        rig.until(|s| s.phase == phase).await?;
        let history = rig.state().history;
        rig.send(Command::LeaveVoice).await?;
        let state = rig
            .until(|s| s.phase == Phase::Ready && !s.voice_connected)
            .await?;
        assert_eq!(rig.queued(), [B]);
        assert_eq!(state.history, history);
        assert_eq!(state.volume, 37);
        assert_eq!(state.preferences.repeat, RepeatMode::Queue);
        assert!(state.preferences.continue_on_error);
        if matches!(phase, Phase::Playing | Phase::Paused) {
            assert!(rig.fake.track(0)?.stopped.load(SeqCst));
            rig.fake.track(0)?.end(); // Stale end must never advance/repeat.
        } else {
            assert_eq!(rig.fake.cancelled.load(SeqCst), 1);
        }
        let requests = rig.fake.requests();
        tokio::time::advance(Duration::from_secs(120)).await;
        rig.settle().await;
        assert_eq!(rig.fake.requests(), requests);
        let connects = rig.fake.connects.load(SeqCst);
        rig.fake.connect_mode.store(0, SeqCst);
        let before = rig.fake.started();
        rig.send(if phase == Phase::Paused {
            Command::Play(C.into())
        } else {
            Command::Next
        })
        .await?;
        rig.started(before + 1).await?;
        assert_eq!(rig.fake.connects.load(SeqCst), connects + 1);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn leave_cancels_scheduled_skip_rejoin_and_inflight_rejoin() -> Result<()> {
    for recovery in 0..3 {
        let mut rig = Rig::start(Saved {
            continue_on_error: true,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        rig.enqueue(A).await?;
        rig.enqueue(B).await?;
        if recovery == 0 {
            rig.fake.fail(A);
            rig.send(Command::Next).await?;
        } else {
            rig.fake.health.send_modify(|health| {
                health.voice = false;
                health.voice_retry = true;
                health.voice_error = Some("Transient voice failure".into());
            });
        }
        rig.until(|s| s.phase == Phase::Error).await?;
        if recovery == 2 {
            rig.fake.connect_mode.store(2, SeqCst);
            tokio::time::advance(Duration::from_secs(1)).await;
            eventually(|| rig.fake.connects.load(SeqCst) == 1).await?;
        }
        rig.send(Command::LeaveVoice).await?;
        rig.until(|s| s.phase == Phase::Ready && !s.voice_connected)
            .await?;
        lose_voice(&rig.fake, true); // A late callback cannot restart recovery after leave.
        let requests = rig.fake.requests();
        let connects = rig.fake.connects.load(SeqCst);
        tokio::time::advance(Duration::from_secs(120)).await;
        rig.settle().await;
        assert!(rig.state().error.is_none());
        assert_eq!(rig.fake.requests(), requests);
        assert_eq!(rig.fake.connects.load(SeqCst), connects);
        assert_ne!(rig.queued(), Vec::<String>::new());
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn disconnect_failure_or_timeout_never_claims_idle_and_can_be_retried() -> Result<()> {
    for hung in [false, true] {
        let mut rig = Rig::new().await?;
        rig.fake.disconnect_failing.store(!hung, SeqCst);
        rig.fake.disconnect_hung.store(hung, SeqCst);
        rig.send(Command::LeaveVoice).await?;
        if hung {
            let state = rig.until(|s| s.phase == Phase::Disconnecting).await?;
            assert!(state.voice_connected);
            assert!(!state.can_play() && !state.can_next() && !state.can_leave_voice());
            tokio::time::advance(Duration::from_secs(5)).await;
        }
        let state = rig.until(|s| s.phase == Phase::Error).await?;
        assert!(state.voice_connected && state.can_leave_voice());
        assert!(
            state
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Could not leave"))
        );
        tokio::time::advance(IDLE_TIMEOUT * 2).await;
        rig.settle().await;
        assert_eq!(rig.fake.disconnects.load(SeqCst), 1);
        rig.fake.disconnect_failing.store(false, SeqCst);
        rig.fake.disconnect_hung.store(false, SeqCst);
        rig.send(Command::LeaveVoice).await?;
        rig.until(|s| s.phase == Phase::Ready && !s.voice_connected)
            .await?;
        rig.send(Command::LeaveVoice).await?; // Already idle remains safe.
        rig.until(|s| s.phase == Phase::Ready && !s.voice_connected)
            .await?;
        assert_eq!(rig.fake.requests(), Vec::<String>::new());
    }
    Ok(())
}

#[tokio::test]
async fn slow_title_lookup_remains_independent_of_play_and_stop() -> Result<()> {
    let mut rig = Rig::new().await?;
    let reply = rig.fake.title_reply(A);
    rig.enqueue(A).await?;
    eventually(|| rig.fake.title_requests() == [A]).await?;
    rig.play(C).await?;
    rig.started(1).await?;
    assert!(!reply.is_closed());
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert!(!reply.is_closed());
    reply
        .send(Ok(Some("Still queued".into())))
        .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
    rig.until(|s| {
        s.queue
            .first()
            .is_some_and(|item| item.label() == "Still queued")
    })
    .await?;
    assert_eq!(rig.fake.requests(), [C]);
    assert_eq!(rig.queued(), [A]);
    Ok(())
}

#[test]
fn leave_voice_availability_follows_session_and_controller_state() {
    let mut state = State {
        available: true,
        controller_ready: true,
        ..State::default()
    };
    for phase in [
        Phase::Connecting,
        Phase::Resolving,
        Phase::Playing,
        Phase::Paused,
        Phase::Error,
        Phase::Ready,
    ] {
        state.phase = phase;
        state.voice_connected = true;
        assert!(state.can_leave_voice(), "{phase:?}");
    }
    for phase in [Phase::Initializing, Phase::Stopping, Phase::Disconnecting] {
        state.phase = phase;
        assert!(!state.can_leave_voice(), "{phase:?}");
    }
    state.phase = Phase::Ready;
    state.voice_connected = false;
    assert!(!state.can_leave_voice());
    state.phase = Phase::Resolving;
    assert!(
        state.can_leave_voice(),
        "pending preparation can be cancelled"
    );
    state.controller_ready = false;
    assert!(!state.can_leave_voice());
}

mod playlist;
