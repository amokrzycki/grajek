//! Media-session intents validated against the authoritative controller state.
use super::{Command, Phase, State};
use crate::media::TrackInfo;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug)]
pub enum MediaCommand {
    Play,
    Pause,
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

impl State {
    pub fn media_track(&self) -> Option<&TrackInfo> {
        self.visible_track()
            .filter(|_| self.available && self.voice_connected)
    }

    pub fn media_status(&self) -> Status {
        if self.media_track().is_none() {
            Status::Stopped
        } else if self.phase == Phase::Paused {
            Status::Paused
        } else {
            Status::Playing
        }
    }

    pub fn media_can_play(&self) -> bool {
        (self.media_track().is_some() && self.can_resume())
            || (self.track.is_none()
                && matches!(self.phase, Phase::Ready | Phase::Error)
                && self.can_next())
    }

    pub fn media_can_pause(&self) -> bool {
        self.media_track().is_some() && self.can_pause()
    }

    pub fn media_can_seek(&self) -> bool {
        self.media_track().is_some() && self.can_seek()
    }

    pub fn media_track_id(&self) -> Option<u64> {
        self.media_track().map(|_| self.track_id)
    }

    pub fn media_duration(&self) -> Option<Duration> {
        self.media_track().and_then(|track| track.duration)
    }

    pub fn media_position(&self) -> Duration {
        self.media_track().map_or(Duration::ZERO, |_| self.position)
    }
}

pub fn translate(command: MediaCommand, state: &State) -> Option<Command> {
    match command {
        MediaCommand::Play | MediaCommand::PlayPause
            if state.media_status() == Status::Paused && state.media_can_play() =>
        {
            Some(Command::Resume)
        }
        MediaCommand::Play | MediaCommand::PlayPause if state.media_can_play() => {
            Some(Command::Next)
        }
        MediaCommand::Pause | MediaCommand::PlayPause if state.media_can_pause() => {
            Some(Command::Pause)
        }
        MediaCommand::Next if state.can_next() => Some(Command::Next),
        MediaCommand::Stop if state.can_stop() => Some(Command::Stop),
        MediaCommand::Seek { track_id, position }
            if state.media_can_seek()
                && state.media_track_id() == Some(track_id)
                && state
                    .media_duration()
                    .is_some_and(|duration| position <= duration) =>
        {
            Some(Command::Seek { track_id, position })
        }
        MediaCommand::SeekRelative { track_id, offset }
            if state.media_can_seek() && state.media_track_id() == Some(track_id) =>
        {
            let delta = Duration::from_micros(offset.unsigned_abs());
            let position = if offset < 0 {
                state.media_position().saturating_sub(delta)
            } else {
                state.media_position().saturating_add(delta)
            };
            if state
                .media_duration()
                .is_some_and(|duration| position > duration)
            {
                state.can_next().then_some(Command::Next)
            } else {
                Some(Command::Seek { track_id, position })
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
