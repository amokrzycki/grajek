mod favorites;
mod import;
mod jobs;
mod playback;
mod queue;
mod rejoin;
mod search;
mod seek;
mod setup;
mod view;
use import::{ImportJob, import_result};
use jobs::{Job, event as job_event};
use playback::{Active, track_outcome};
use queue::{TitleJob, title_result};
use seek::{SeekJob, seek_result};
use setup::SetupChange;

use crate::favorites::FavoriteResult;
use crate::playlist::ImportStatus;
use crate::queue::Queue;
use crate::{discord::Health, store::Saved};
use anyhow::{Context, Result};
use std::{future::pending, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval, sleep_until, timeout},
};

use super::{Backend, Command, Handle, Phase, State, VolumeRequest};

const IDLE_TIMEOUT: Duration = Duration::from_mins(10);

/// Whole-second position: reported values never split a second across ticks.
pub(super) fn whole_seconds(position: Duration) -> Duration {
    Duration::from_secs(position.as_secs())
}

#[derive(Clone, Copy)]
enum Scheduled {
    Advance,
    Rejoin(u8),
    Idle,
}

/// All playback, queue and volume state lives here. The GUI never mutates a copy.
pub(super) struct Controller<B: Backend> {
    setup_job: Option<JoinHandle<Result<()>>>,
    setup_change: SetupChange,
    pending_setup: Option<crate::discord::setup::Request>,
    backend: Arc<B>,
    health: watch::Receiver<Health>,
    states: watch::Sender<State>,
    persist: watch::Sender<Saved>,
    saved: Saved,
    queue: Queue,
    volume: u8,
    volume_seq: u64,
    transport_ack: u64,
    enqueue_result: Option<super::EnqueueResult>,
    components_request: u64,
    notice: Option<String>,
    saver_status: Option<watch::Receiver<Option<&'static str>>>,
    job: Option<Job>,
    track: Option<Active<B::Handle>>,
    failures: u8,
    failure_notice: Option<String>,
    scheduled: Option<(Instant, Scheduled)>,
    rejoin_attempts: u8,
    left_voice: bool,
    title_job: Option<TitleJob>,
    import_job: Option<ImportJob>,
    import: ImportStatus,
    seek_job: Option<SeekJob>,
    track_id: u64,
    search: search::Search,
    favorite_result: Option<FavoriteResult>,
}

impl<B: Backend> Controller<B> {
    pub(super) fn new(
        backend: Arc<B>,
        saved: Saved,
        notice: Option<String>,
        states: watch::Sender<State>,
        persist: watch::Sender<Saved>,
        saver_status: Option<watch::Receiver<Option<&'static str>>>,
    ) -> Self {
        Self {
            setup_job: None,
            setup_change: SetupChange::Discovery,
            pending_setup: None,
            health: backend.health(),
            backend,
            states,
            persist,
            volume: saved.volume,
            queue: saved.restored_queue(),
            saved,
            volume_seq: 0,
            transport_ack: 0,
            enqueue_result: None,
            components_request: 0,
            notice,
            saver_status,
            job: None,
            track: None,
            failures: 0,
            failure_notice: None,
            scheduled: None,
            rejoin_attempts: 0,
            left_voice: false,
            title_job: None,
            import_job: None,
            import: ImportStatus::Ready,
            seek_job: None,
            track_id: 0,
            search: search::Search::default(),
            favorite_result: None,
        }
    }

    fn gateway(&self) -> bool {
        self.health.borrow().gateway && self.backend.ready() && !self.setup_interrupting()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One exhaustive dispatcher preserves transport cancellation and ordering"
    )]
    async fn command(&mut self, command: Command) {
        let mut command = command;
        let mut acknowledged = false;
        loop {
            match command {
                Command::Transport { id, command: inner } => {
                    self.transport_ack = id;
                    acknowledged = true;
                    command = *inner;
                }
                Command::Media(request) => {
                    let Some(inner) = super::media::translate(request, &self.states.borrow())
                    else {
                        return self.sync();
                    };
                    acknowledged = true;
                    command = inner;
                }
                Command::CloseToTray(enabled) => {
                    self.set_close_to_tray(enabled);
                    break;
                }
                Command::Setup(request) => {
                    self.setup_command(request).await;
                    break;
                }
                Command::ConfirmSetup(accept) => {
                    self.confirm_setup(accept).await;
                    break;
                }
                Command::Seek { track_id, position } => {
                    self.seek(track_id, position);
                    break;
                }
                Command::ImportPlaylist(value) => {
                    self.start_import(&value);
                    break;
                }
                Command::CancelImport => {
                    self.cancel_import_command().await;
                    break;
                }
                Command::Search { id, query } => {
                    self.submit_search(id, query);
                    break;
                }
                Command::CancelSearch { id } => {
                    self.cancel_search(id);
                    break;
                }
                Command::SearchAction {
                    search_id,
                    video_id,
                    action,
                } => {
                    self.search_action(search_id, &video_id, action).await;
                    break;
                }
                Command::SetFavorite { id, url, saved } => {
                    self.set_favorite(id, &url, saved);
                    break;
                }
                Command::LeaveVoice => {
                    self.leave_voice_command().await;
                    break;
                }
                Command::Stop => {
                    self.stop_command().await;
                    break;
                }
                Command::Pause => {
                    self.set_paused(true);
                    break;
                }
                Command::Resume => {
                    self.set_paused(false);
                    break;
                }
                Command::Play(value) => {
                    self.play_command(&value).await;
                    break;
                }
                Command::EnqueueRequest {
                    id,
                    value,
                    placement,
                } => {
                    self.enqueue_command(id, &value, placement);
                    break;
                }
                Command::MoveToNext { request_id, id } => {
                    self.move_to_next(request_id, id);
                    break;
                }
                Command::Next => {
                    self.next_command().await;
                    break;
                }
                Command::Dequeue(id) => {
                    self.dequeue(id);
                    break;
                }
                Command::Shift { id, up } => {
                    self.shift(id, up);
                    break;
                }
                Command::ClearQueue => {
                    self.clear_queue().await;
                    break;
                }
                Command::ForgetHistory(url) => {
                    self.forget_history(&url);
                    break;
                }
                Command::RememberQueue(enabled) => {
                    self.remember_queue(enabled);
                    break;
                }
                Command::Repeat(mode) => {
                    self.set_repeat(mode);
                    break;
                }
                Command::ContinueOnError(enabled) => {
                    self.continue_on_error(enabled);
                    break;
                }
                Command::ClearHistory => {
                    self.clear_history();
                    break;
                }
            }
        }
        if acknowledged {
            self.sync();
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
                self.job = Some(Job::Rejoin {
                    task: tokio::spawn(async move {
                        timeout(Duration::from_secs(40), backend.connect())
                            .await
                            .context("Voice recovery timed out.")??;
                        Ok(())
                    }),
                    attempt,
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

    pub(super) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut volume: watch::Receiver<VolumeRequest>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut health = self.health.clone();
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
            let search_deadline = self.search.deadline();
            let poll_handle = self
                .track
                .as_ref()
                .filter(|_| self.seek_job.is_none())
                .map(|track| track.handle.clone());
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
                result = async {
                    match &mut self.setup_job { Some(job) => job.await, None => pending().await }
                } => self.finish_setup(&result).await,
                result = search::result(&mut self.search.job) => self.finish_search(result),
                () = async {
                    match search_deadline { Some(deadline) => sleep_until(deadline).await, None => pending().await }
                } => self.search_expired(),
                result = import_result(&mut self.import_job) => self.finish_import(result),
                result = title_result(&mut self.title_job) => self.finish_title(result),
                changed = async {
                    match &mut self.saver_status { Some(status) => status.changed().await, None => pending().await }
                } => {
                    if changed.is_err() {
                        self.saver_status = None;
                        let warning = "Saving stopped unexpectedly; changes may be lost when Grajek closes.";
                        self.notice = Some(self.notice.as_ref().map_or_else(|| warning.into(), |startup| format!("{warning} {startup}")));
                    }
                    self.sync();
                },
                result = seek_result(&mut self.seek_job) => self.finish_seek(result),
                event = job_event(&mut self.job) => self.finish_job(event).await,
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
                    if self.seek_job.is_none() && self.apply(reading) {
                        self.natural_end().await;
                    }
                },
            }
        }
        self.join_search().await;
        self.cancel_import().await;
        self.cancel_titles().await;
        self.cancel_job().await;
        self.stop_track();
        if let Some(job) = self.setup_job.take() {
            job.abort();
            let _ = job.await;
        }
        self.backend.shutdown().await;
    }
}

#[cfg(test)]
mod tests;
