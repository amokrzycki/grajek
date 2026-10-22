use anyhow::{Context, Result};
use songbird::tracks::{PlayMode, TrackResult, TrackState};
use std::{future::pending, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinError, JoinHandle},
    time::{Instant, MissedTickBehavior, interval, sleep_until, timeout},
};

use super::{
    Backend, Command, Handle, ImportStatus, Outcome, Phase, Preferences, RepeatMode, State,
    VolumeRequest, volume_factor,
};
use crate::{
    config::{playlist_url, video_url},
    discord::Health,
    media::{self, Resolved, TrackInfo},
    queue::{Full, QUEUE_LIMIT, Queue},
    store::{HistoryEntry, Saved, SavedQueueItem},
};

/// How long a track may take to start advancing before it is reported as failed.
const START_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_mins(10);
const SKIP_DELAY: Duration = Duration::from_secs(1);
const FAILURE_LIMIT: u8 = 3;
const REJOIN_LIMIT: u8 = 2;
const TITLE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
enum Scheduled {
    Advance,
    Rejoin(u8),
    Idle,
}

enum JobKind {
    Prepare { url: String, queued: bool },
    Rejoin(u8),
    Disconnect { manual: bool },
}

enum JobOutput {
    Prepared(Resolved),
    Connected,
    Disconnected,
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

/// One cancellable playback or voice job. Recovery and extraction share this slot.
struct Job {
    task: JoinHandle<Result<JobOutput>>,
    connected: Option<oneshot::Receiver<()>>,
    kind: JobKind,
    voice_ready: bool,
}

/// The import owns its task even when the controller future is dropped abruptly.
struct ImportJob {
    task: JoinHandle<Result<crate::playlist::Playlist>>,
}

impl Drop for ImportJob {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn import_result(
    job: &mut Option<ImportJob>,
) -> Result<Result<crate::playlist::Playlist>, JoinError> {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

/// One title subprocess at a time, independent of playback and playlist import.
struct TitleJob {
    url: String,
    ids: Vec<u64>,
    task: JoinHandle<Result<Option<String>>>,
}

async fn title_result(job: &mut Option<TitleJob>) -> Result<Result<Option<String>>, JoinError> {
    match job {
        Some(job) => (&mut job.task).await,
        None => pending().await,
    }
}

enum JobEvent {
    /// Voice is ready (`true`), or the job ended without connecting (`false`).
    Connected(bool),
    Done(Result<Result<JobOutput>, JoinError>),
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
    queued_track: bool,
    failures: u8,
    failure_notice: Option<String>,
    scheduled: Option<(Instant, Scheduled)>,
    rejoin_attempts: u8,
    left_voice: bool,
    title_job: Option<TitleJob>,
    import_job: Option<ImportJob>,
    import: ImportStatus,
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
            queue: saved.restored_queue(),
            saved,
            volume_seq: 0,
            notice,
            job: None,
            track: None,
            queued_track: false,
            failures: 0,
            failure_notice: None,
            scheduled: None,
            rejoin_attempts: 0,
            left_voice: false,
            title_job: None,
            import_job: None,
            import: ImportStatus::Ready,
        }
    }

    fn gateway(&self) -> bool {
        self.health.borrow().gateway
    }

    /// Adds the shared data (volume, queue, history) to a state about the track/phase.
    fn view(&self, mut state: State) -> State {
        state.available = self.gateway();
        state.controller_ready = true;
        state.voice_connected = self.health.borrow().voice;
        state.preferences = Preferences {
            continue_on_error: self.saved.continue_on_error,
            remember_queue: self.saved.remember_queue,
            repeat: self.saved.repeat,
        };
        if state.error.is_none() {
            state.error.clone_from(&self.failure_notice);
        }
        state.volume = self.volume;
        state.volume_seq = self.volume_seq;
        state.queue = self.queue.snapshot();
        state.import.clone_from(&self.import);
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

    fn publish(&self, phase: Phase, error: Option<String>) {
        self.emit(State {
            phase,
            error,
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

    fn queue_changed(&mut self) {
        self.saved.queue = if self.saved.remember_queue {
            self.queue
                .snapshot()
                .into_iter()
                .map(|item| SavedQueueItem {
                    url: item.url,
                    title: item.title.as_deref().and_then(crate::store::safe_title),
                })
                .collect()
        } else {
            Vec::new()
        };
        self.persist();
        self.sync();
    }

    fn known_title(&self, url: &str) -> Option<String> {
        self.saved
            .title_of(url)
            .or_else(|| self.queue.title_of(url))
            .or_else(|| {
                self.track
                    .as_ref()
                    .filter(|track| track.url == url)
                    .and_then(|track| track.info.title.clone())
            })
    }

    async fn refresh_titles(&mut self) {
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

    fn finish_title(&mut self, result: Result<Result<Option<String>>, JoinError>) {
        let Some(job) = self.title_job.take() else {
            return;
        };
        let title = match result {
            Ok(Ok(title)) => title.and_then(|title| crate::store::safe_title(&title)),
            result => {
                tracing::debug!(?result, "Queue title lookup failed");
                None
            }
        };
        self.queue.resolve_titles(&job.ids, title.as_deref());
        self.queue_changed();
    }

    fn start_import(&mut self, value: &str) {
        if self.import_job.is_some() {
            return self.sync();
        }
        match playlist_url(value) {
            Err(error) => self.import = ImportStatus::Failed(error.to_string()),
            Ok(_) if self.queue.snapshot().len() >= QUEUE_LIMIT => {
                self.import = ImportStatus::Failed(
                    "Queue full (100 tracks). Remove entries and try again.".into(),
                );
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

    async fn cancel_import(&mut self) {
        if let Some(mut job) = self.import_job.take() {
            job.task.abort();
            let _ = (&mut job.task).await;
            self.import = ImportStatus::Cancelled;
        }
    }

    fn finish_import(&mut self, result: Result<Result<crate::playlist::Playlist>, JoinError>) {
        if self.import_job.take().is_none() {
            return;
        }
        match result {
            Ok(Ok(playlist)) => {
                let total = playlist.entries.len();
                let added = self.queue.append(playlist.entries);
                self.import = if added == 0 {
                    ImportStatus::Failed(
                        "Queue full (100 tracks). Remove entries and try again.".into(),
                    )
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
                self.import = ImportStatus::Failed(match error {
                    Ok(Err(error)) => error.downcast_ref::<crate::playlist::Error>().map_or_else(
                        || "Playlist unavailable. Check the public link and try again.".into(),
                        ToString::to_string,
                    ),
                    _ => "Playlist import failed. Try again.".into(),
                });
                self.sync();
            }
        }
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
        self.publish(Phase::Error, Some(message.into()));
    }

    /// Starts `url`, cancelling whatever was being prepared or played. Only one request
    /// and one track can exist because everything runs through this single path.
    async fn begin(&mut self, url: String, queued: bool) {
        self.left_voice = false;
        self.scheduled = None;
        self.queued_track = queued;
        self.cancel_job().await;
        self.stop_track();
        self.backend.clear_error();
        self.publish(Phase::Connecting, None);
        let (connected_tx, connected) = oneshot::channel();
        let backend = self.backend.clone();
        let request = url.clone();
        self.job = Some(Job {
            task: tokio::spawn(async move {
                backend
                    .prepare(request, connected_tx)
                    .await
                    .map(JobOutput::Prepared)
            }),
            connected: Some(connected),
            kind: JobKind::Prepare { url, queued },
            voice_ready: false,
        });
    }

    /// Called exactly once per track that ended on its own: the next queued track, or Ready.
    async fn advance(&mut self) {
        let next = self.queue.pop_front();
        self.queue_changed();
        match next {
            Some(item) => self.begin(item.url, true).await,
            None => self.publish(Phase::Ready, None),
        }
    }

    /// Only a confirmed natural completion can recycle media. Errors and manual
    /// cancellation never enter this path; each successor gets fresh extraction.
    async fn natural_end(&mut self) {
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
        self.stop_track();
        match self.saved.repeat {
            RepeatMode::Track => self.begin(completed.url, self.queued_track).await,
            RepeatMode::Queue if self.queued_track => {
                // Pop first: even 100 newly pending entries leave space for rotation.
                let next = self.queue.pop_front();
                if let Some(next) = next {
                    if self.queue.push(completed.url, completed.title).is_err() {
                        self.report("Could not rotate the completed track: queue is full.".into());
                    }
                    self.queue_changed();
                    self.begin(next.url, true).await;
                } else {
                    self.begin(completed.url, true).await;
                }
            }
            RepeatMode::Off | RepeatMode::Queue => self.advance().await,
        }
    }

    fn enqueue(&mut self, value: &str) {
        match video_url(value) {
            Ok(url) => {
                let title = self.known_title(&url);
                match self.queue.push(url, title) {
                    Ok(_) => self.queue_changed(),
                    Err(Full) => {
                        self.report(format!("The queue is full ({QUEUE_LIMIT} tracks)."));
                    }
                }
            }
            Err(error) => self.report(error.to_string()),
        }
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::ImportPlaylist(value) => self.start_import(&value),
            Command::CancelImport => {
                self.cancel_import().await;
                self.sync();
            }
            Command::LeaveVoice => self.leave_voice(true).await,
            Command::Stop => {
                if !matches!(
                    self.job.as_ref().map(|job| &job.kind),
                    Some(JobKind::Disconnect { manual: true })
                ) {
                    self.stop().await;
                }
            }
            Command::Pause => self.set_paused(true),
            Command::Resume => self.set_paused(false),
            Command::Play(value) => {
                if matches!(
                    self.job.as_ref().map(|job| &job.kind),
                    Some(JobKind::Prepare { .. } | JobKind::Disconnect { manual: true })
                ) || !self.gateway()
                {
                    return self.sync();
                }
                match video_url(&value) {
                    Ok(url) => {
                        self.failure_notice = None;
                        self.rejoin_attempts = 0;
                        self.begin(url, false).await;
                    }
                    // A bad URL must not disturb the track that is still playing or paused.
                    Err(error) => self.report(error.to_string()),
                }
            }
            Command::Enqueue(value) => self.enqueue(&value),
            Command::Next => {
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
            Command::Dequeue(id) => {
                self.queue.remove(id);
                self.queue_changed();
            }
            Command::Shift { id, up } => {
                self.queue.shift(id, up);
                self.queue_changed();
            }
            Command::ClearQueue => {
                self.cancel_import().await;
                self.queue.clear();
                self.queue_changed();
            }
            Command::ForgetHistory(url) => {
                if self.saved.remove(&url) {
                    self.persist();
                }
                self.sync();
            }
            Command::RememberQueue(enabled) => {
                self.saved.remember_queue = enabled;
                self.queue_changed();
            }
            Command::Repeat(mode) => {
                self.saved.repeat = mode;
                self.persist();
                self.sync();
            }
            Command::ContinueOnError(enabled) => {
                self.saved.continue_on_error = enabled;
                if !enabled && matches!(self.scheduled, Some((_, Scheduled::Advance))) {
                    self.scheduled = None;
                    self.report("Automatic continuation disabled. Press Next or Play URL.".into());
                }
                self.persist();
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

    async fn leave_voice(&mut self, manual: bool) {
        if matches!(
            self.job.as_ref().map(|job| &job.kind),
            Some(JobKind::Disconnect { manual: true })
        ) {
            return self.sync();
        }
        self.left_voice = true;
        self.scheduled = None;
        self.failure_notice = None;
        self.rejoin_attempts = 0;
        self.cancel_job().await;
        self.stop_track();
        // Automatic idle leave remains interruptible by explicit Play/Next.
        if manual {
            self.publish(Phase::Disconnecting, None);
        }
        let backend = self.backend.clone();
        self.job = Some(Job {
            task: tokio::spawn(async move {
                timeout(Duration::from_secs(5), backend.disconnect())
                    .await
                    .context("Leaving voice timed out.")??;
                Ok(JobOutput::Disconnected)
            }),
            connected: None,
            kind: JobKind::Disconnect { manual },
            voice_ready: false,
        });
    }

    /// Cancels pending work and the active track. The queue is kept and nothing advances.
    async fn stop(&mut self) {
        self.scheduled = None;
        self.failure_notice = None;
        self.publish(Phase::Stopping, None);
        self.cancel_job().await;
        self.stop_track();
        self.backend.recover().await;
        let error = self.health.borrow().error();
        let phase = if error.is_some() {
            Phase::Error
        } else {
            Phase::Ready
        };
        self.publish(phase, error);
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
                    self.failures = 0;
                    if let Some(entry) = entry {
                        self.saved.record(entry);
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
            self.media_failure(message);
        }
        false
    }

    fn media_failure(&mut self, message: &str) {
        self.fail(message);
        let healthy = {
            let health = self.health.borrow();
            health.gateway && health.voice && health.error().is_none()
        };
        if !self.saved.continue_on_error || !self.queued_track || !healthy {
            return;
        }
        self.failures = self.failures.saturating_add(1);
        let explanation = if self.failures >= FAILURE_LIMIT {
            format!(
                "Recovery stopped after {FAILURE_LIMIT} consecutive failures. Press Next or Play URL."
            )
        } else if self.queue.is_empty() {
            "No queued tracks remain. Press Play URL or add tracks and press Next.".into()
        } else {
            self.scheduled = Some((Instant::now() + SKIP_DELAY, Scheduled::Advance));
            "Skipped failed queued track; continuing in 1 second.".into()
        };
        self.failure_notice = Some(format!("{message} {explanation}"));
        self.publish(Phase::Error, self.failure_notice.clone());
    }

    fn schedule_rejoin(&mut self) {
        if self.rejoin_attempts >= REJOIN_LIMIT {
            self.publish(Phase::Error, Some("Discord voice recovery stopped after 2 attempts. Check the network and permissions, then press Next or Play URL.".into()));
        } else {
            self.rejoin_attempts += 1;
            self.scheduled = Some((
                Instant::now() + Duration::from_secs(u64::from(self.rejoin_attempts) * 2 - 1),
                Scheduled::Rejoin(self.rejoin_attempts),
            ));
            self.publish(Phase::Error, Some("Discord voice interrupted; reconnecting. Playback will not restart automatically.".into()));
        }
    }

    async fn health_changed(&mut self, current: Health) {
        // Explicit leave owns teardown; late voice callbacks cannot cancel it or rejoin.
        if self.left_voice {
            if current.gateway_error.is_some() && self.states.borrow().phase != Phase::Disconnecting
            {
                self.publish(Phase::Error, current.gateway_error);
            } else {
                self.sync();
            }
            return;
        }
        let Some(error) = current.error() else {
            self.sync();
            return;
        };
        // Voice callbacks from a rejoin belong to that job; its result owns the retry budget.
        if current.gateway
            && current.voice_retry
            && (matches!(
                self.job.as_ref().map(|job| &job.kind),
                Some(JobKind::Rejoin(_))
            ) || matches!(self.scheduled, Some((_, Scheduled::Rejoin(_)))))
        {
            return;
        }
        self.scheduled = None;
        self.cancel_job().await;
        self.stop_track();
        self.failure_notice = None;
        self.publish(Phase::Error, Some(error));
        if current.gateway && !current.voice && current.voice_retry {
            self.schedule_rejoin();
        }
    }

    async fn scheduled_work(&mut self) {
        let Some((_, work)) = self.scheduled.take() else {
            return;
        };
        match work {
            Scheduled::Advance => {
                if self.queue.is_empty() {
                    self.failure_notice = Some("Automatic continuation stopped: the queue is empty. Press Play URL to start playback.".into());
                    self.publish(Phase::Error, self.failure_notice.clone());
                    return;
                }
                if self.saved.continue_on_error
                    && self.gateway()
                    && self.health.borrow().voice
                    && self.health.borrow().error().is_none()
                {
                    self.advance().await;
                }
            }
            Scheduled::Rejoin(attempt) => {
                if !self.gateway() {
                    return;
                }
                self.backend.clear_error();
                let backend = self.backend.clone();
                self.job = Some(Job {
                    task: tokio::spawn(async move {
                        timeout(Duration::from_secs(40), backend.connect())
                            .await
                            .context("Voice recovery timed out.")??;
                        Ok(JobOutput::Connected)
                    }),
                    connected: None,
                    kind: JobKind::Rejoin(attempt),
                    voice_ready: false,
                });
            }
            Scheduled::Idle => {
                if self.job.is_some() || self.track.is_some() || !self.health.borrow().voice {
                    return;
                }
                self.leave_voice(false).await;
            }
        }
    }

    async fn finish_job(&mut self, result: Result<Result<JobOutput>, JoinError>) {
        let Some(job) = self.job.take() else {
            return;
        };
        let result = result
            .context("Audio request task failed.")
            .and_then(|result| result);
        match (job.kind, result) {
            (JobKind::Prepare { url, queued }, Ok(JobOutput::Prepared(resolved))) => {
                self.queued_track = queued;
                let info = resolved.info.clone();
                match timeout(
                    Duration::from_secs(2),
                    self.backend.play(resolved, volume_factor(self.volume)),
                )
                .await
                {
                    Ok(Ok((handle, outcome))) => {
                        let current = Active::new(handle, outcome, info, url);
                        self.emit(current.state(None));
                        self.track = Some(current);
                    }
                    Ok(Err(error)) => {
                        tracing::error!(error = %format!("{error:#}"), "Could not start audio playback");
                        self.media_failure("Could not start audio playback. Try again.");
                    }
                    Err(_) => self
                        .media_failure("Audio playback startup timed out. Press Next or Play URL."),
                }
            }
            (JobKind::Prepare { queued, .. }, Err(error)) => {
                tracing::error!(error = %format!("{error:#}"), "Audio request failed");
                self.queued_track = queued && job.voice_ready;
                self.media_failure(&media::user_error(&error));
                self.backend.recover().await;
            }
            (JobKind::Rejoin(attempt), result) => {
                self.rejoin_attempts = attempt;
                if result.is_ok() && self.health.borrow().voice {
                    self.rejoin_attempts = 0;
                    self.publish(
                        Phase::Ready,
                        Some(
                            "Discord voice restored. Press Next or Play URL to start playback."
                                .into(),
                        ),
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
            (JobKind::Disconnect { .. }, result) => {
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
            _ => self.fail("Unexpected playback job result. Press Next or Play URL."),
        }
    }

    pub(super) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut volume: watch::Receiver<VolumeRequest>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut health = self.health.clone();
        // Poll only while a track exists; otherwise sleep until the single scheduled deadline.
        let mut tick = interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        self.publish(Phase::Ready, None);
        loop {
            self.refresh_titles().await;
            if !self.left_voice
                && self.scheduled.is_none()
                && self.job.is_none()
                && self.track.is_none()
                && self.health.borrow().voice
            {
                self.scheduled = Some((Instant::now() + IDLE_TIMEOUT, Scheduled::Idle));
            }
            let deadline = self.scheduled.map(|(deadline, _)| deadline);
            let poll_handle = self.track.as_ref().map(|track| track.handle.clone());
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
                    self.health_changed(current).await;
                },
                result = import_result(&mut self.import_job) => self.finish_import(result),
                result = title_result(&mut self.title_job) => self.finish_title(result),
                event = job_event(&mut self.job) => match event {
                    JobEvent::Connected(reached) => {
                        if let Some(job) = &mut self.job { job.voice_ready = reached; }
                        if reached && self.states.borrow().phase == Phase::Connecting {
                            self.publish(Phase::Resolving, None);
                        }
                    }
                    JobEvent::Done(result) => self.finish_job(result).await,
                },
                alive = track_outcome(&mut self.track) => {
                    if self.handle_outcome(alive) {
                        self.natural_end().await;
                    }
                },
                () = async {
                    match deadline {
                        Some(deadline) => sleep_until(deadline).await,
                        None => pending().await,
                    }
                } => self.scheduled_work().await,
                reading = async {
                    let Some(handle) = poll_handle else { return pending().await; };
                    tick.tick().await;
                    timeout(Duration::from_secs(2), handle.info()).await.ok()
                } => {
                    if self.apply(reading) {
                        self.natural_end().await;
                    }
                },
            }
        }
        self.cancel_import().await;
        if let Some(job) = self.title_job.take() {
            job.task.abort();
            let _ = job.task.await;
        }
        self.cancel_job().await;
        self.stop_track();
        self.backend.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
