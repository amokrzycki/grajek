use super::{Controller, Scheduled};
use crate::history;
use crate::player::{Backend, Phase, SeekStatus, State};
use crate::preferences::{Preferences, RepeatMode};

impl<B: Backend> Controller<B> {
    pub(super) fn view(&self, mut state: State) -> State {
        state.close_to_tray = self.saved.close_to_tray;
        state.transport_ack = self.transport_ack;
        state.enqueue_result = self.enqueue_result;
        state.components_request = self.components_request;
        state.track_id = self.track.as_ref().map_or(0, |track| track.id);
        state.seek = self
            .track
            .as_ref()
            .map_or(SeekStatus::Ready, |track| track.seek.clone());
        state.seek_serial = self.track.as_ref().map_or(0, |track| track.seek_serial);
        state.available = self.gateway();
        state.setup = self.health.borrow().setup.clone();
        state.setup_busy = self.setup_job.is_some();
        state.confirm_setup = self.pending_setup.is_some();
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
        state.search.clone_from(&self.search.state);
        state.history.clone_from(&self.saved.history);
        state.favorites.clone_from(&self.saved.favorites);
        state.favorite_result.clone_from(&self.favorite_result);
        state.current_url = self
            .track
            .as_ref()
            .map(|track| track.request.url.clone())
            .or_else(|| {
                self.job
                    .as_ref()
                    .and_then(|job| job.current_url().map(str::to_owned))
            });
        let save_notice = self
            .saver_status
            .as_ref()
            .and_then(|status| *status.borrow());
        state.notice = match (&self.notice, save_notice) {
            (Some(startup), Some(runtime)) => Some(format!("{runtime} {startup}")),
            (Some(startup), None) => Some(startup.clone()),
            (None, Some(runtime)) => Some(runtime.into()),
            (None, None) => None,
        };
        state
    }

    pub(super) fn emit(&self, state: State) {
        let state = self.view(state);
        if self.states.borrow().phase != state.phase {
            tracing::info!(state = ?state.phase, "Player state");
        }
        self.states.send_replace(state);
    }

    pub(super) fn publish(&self, phase: Phase, error: Option<String>) {
        self.emit(State {
            phase,
            error,
            ..State::default()
        });
    }

    /// Refreshes the shared data and always answers, so no GUI action is left waiting.
    pub(super) fn sync(&self) {
        self.states
            .send_modify(|state| *state = self.view(std::mem::take(state)));
    }

    /// Shows a message without disturbing what is playing or being prepared.
    pub(super) fn report(&self, message: String) {
        self.states.send_modify(|state| {
            state.error = Some(message);
            if state.phase == Phase::Ready {
                state.phase = Phase::Error;
            }
        });
    }

    pub(super) fn persist(&self) {
        self.persist.send_replace(self.saved.clone());
    }

    pub(super) fn set_close_to_tray(&mut self, enabled: bool) {
        self.saved.close_to_tray = enabled;
        self.persist();
        self.sync();
    }

    pub(super) fn forget_history(&mut self, url: &str) {
        if history::remove(&mut self.saved.history, url) {
            self.persist();
        }
        self.sync();
    }

    pub(super) fn remember_queue(&mut self, enabled: bool) {
        self.saved.remember_queue = enabled;
        self.queue_changed();
    }

    pub(super) fn set_repeat(&mut self, mode: RepeatMode) {
        self.saved.repeat = mode;
        self.persist();
        self.sync();
    }

    pub(super) fn continue_on_error(&mut self, enabled: bool) {
        self.saved.continue_on_error = enabled;
        if !enabled && matches!(self.scheduled, Some((_, Scheduled::Advance))) {
            self.scheduled = None;
            self.report("Automatic continuation disabled. Press Next or Play URL.".into());
        }
        self.persist();
        self.sync();
    }

    pub(super) fn clear_history(&mut self) {
        if !self.saved.history.is_empty() {
            self.saved.history.clear();
            self.persist();
        }
        self.sync();
    }
}
