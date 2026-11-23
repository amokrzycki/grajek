//! Saved playback preferences, independent of transient connection state.

/// Loudest bot volume, in percent.
pub const VOLUME_MAX: u8 = 100;

/// Playback is never amplified above [`VOLUME_MAX`].
pub fn clamp_volume(percent: u8) -> u8 {
    percent.min(VOLUME_MAX)
}

/// Completion policy; preferences from older releases default to ordinary FIFO.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    Track,
    Queue,
    #[default]
    #[serde(other)]
    Off,
}

impl RepeatMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Repeat: Off",
            Self::Track => "Repeat Track",
            Self::Queue => "Repeat Queue",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Track,
            Self::Track => Self::Queue,
            Self::Queue => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub continue_on_error: bool,
    pub remember_queue: bool,
    pub repeat: RepeatMode,
}
