//! OS sessions observe controller snapshots and submit bounded intents; they own no playback.
use crate::player::{self, Phase, State};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Play,
    Pause,
    #[cfg_attr(
        all(target_os = "windows", not(test)),
        expect(
            dead_code,
            reason = "SMTC emits separate Play and Pause events; MPRIS also emits PlayPause"
        )
    )]
    PlayPause,
    Next,
    Stop,
    Seek {
        track_id: u64,
        position: Duration,
    },
    #[cfg_attr(
        all(target_os = "windows", not(test)),
        expect(
            dead_code,
            reason = "Relative seek is exposed by MPRIS; SMTC supplies absolute positions"
        )
    )]
    SeekRelative {
        track_id: u64,
        offset: i64,
    },
}

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Playing,
    Paused,
}

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
        let track = state
            .visible_track()
            .filter(|_| state.available && state.voice_connected);
        Self {
            status: if track.is_none() {
                Status::Stopped
            } else if state.phase == Phase::Paused {
                Status::Paused
            } else {
                Status::Playing
            },
            track_id: track.map(|_| state.track_id),
            title: track.and_then(|track| track.title.clone()),
            duration: track.and_then(|track| track.duration),
            position: track.map_or(Duration::ZERO, |_| state.position),
            seek_serial: state.seek_serial,
            can_play: (track.is_some() && state.can_resume())
                || (state.track.is_none()
                    && matches!(state.phase, Phase::Ready | Phase::Error)
                    && state.can_next()),
            can_pause: track.is_some() && state.can_pause(),
            can_seek: track.is_some() && state.can_seek(),
            can_next: state.can_next(),
            can_stop: state.can_stop(),
            volume: state.volume,
        }
    }
}

/// Evaluated by the controller at consumption, so duplicate Play cannot skip a queue item.
pub fn translate(command: Command, state: &State) -> Option<player::Command> {
    let snapshot = Snapshot::from(state);
    match command {
        Command::Play | Command::PlayPause
            if snapshot.status == Status::Paused && snapshot.can_play =>
        {
            Some(player::Command::Resume)
        }
        Command::Play | Command::PlayPause if snapshot.can_play => Some(player::Command::Next),
        Command::Pause | Command::PlayPause if snapshot.can_pause => Some(player::Command::Pause),
        Command::Next if snapshot.can_next => Some(player::Command::Next),
        Command::Stop if snapshot.can_stop => Some(player::Command::Stop),
        Command::Seek { track_id, position }
            if snapshot.can_seek
                && snapshot.track_id == Some(track_id)
                && snapshot
                    .duration
                    .is_some_and(|duration| position <= duration) =>
        {
            Some(player::Command::Seek { track_id, position })
        }
        Command::SeekRelative { track_id, offset }
            if snapshot.can_seek && snapshot.track_id == Some(track_id) =>
        {
            let delta = Duration::from_micros(offset.unsigned_abs());
            let position = if offset < 0 {
                snapshot.position.saturating_sub(delta)
            } else {
                snapshot.position.saturating_add(delta)
            };
            if snapshot
                .duration
                .is_some_and(|duration| position > duration)
            {
                snapshot.can_next.then_some(player::Command::Next)
            } else {
                Some(player::Command::Seek { track_id, position })
            }
        }
        _ => None,
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

    pub fn send(&self, command: Command) -> Result<(), &'static str> {
        self.commands
            .try_send(player::Command::Media(command))
            .map_err(|_| "Player command channel is busy or closed")
    }
}
