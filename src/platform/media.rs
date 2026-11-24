//! OS sessions observe controller snapshots and submit bounded intents; they own no playback.
use crate::player::{self, MediaCommand, State};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

pub use crate::player::Status;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "OS transport capabilities are independent controller facts"
)]
pub struct Snapshot {
    pub status: Status,
    pub track_id: Option<u64>,
    pub title: Option<String>,
    pub duration: Option<Duration>,
    pub position: Duration,
    pub seek_serial: u64,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub can_next: bool,
    pub can_stop: bool,
    pub volume: u8,
}

impl From<&State> for Snapshot {
    fn from(state: &State) -> Self {
        Self {
            status: state.media_status(),
            track_id: state.media_track_id(),
            title: state.media_track().and_then(|track| track.title.clone()),
            duration: state.media_duration(),
            position: state.media_position(),
            seek_serial: state.seek_serial,
            can_play: state.media_can_play(),
            can_pause: state.media_can_pause(),
            can_seek: state.media_can_seek(),
            can_next: state.can_next(),
            can_stop: state.can_stop(),
            volume: state.volume,
        }
    }
}

impl Snapshot {
    pub fn play_enabled(&self) -> bool {
        self.can_play || self.can_pause
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn pause_enabled(&self) -> bool {
        self.can_pause || self.status == Status::Paused && self.can_play
    }
}

#[derive(Clone)]
pub struct Endpoint {
    pub commands: mpsc::Sender<player::Command>,
    pub states: watch::Receiver<State>,
}

impl Endpoint {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::from(&*self.states.borrow())
    }

    pub fn send(&self, command: MediaCommand) -> Result<(), &'static str> {
        self.commands
            .try_send(player::Command::Media(command))
            .map_err(|_| "Player command channel is busy or closed")
    }
}
