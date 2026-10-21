use std::time::Duration;

use crate::{
    media::TrackInfo,
    queue::QueueItem,
    store::{self, HistoryEntry},
};

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
    pub voice_connected: bool,
    pub continue_on_error: bool,
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
            voice_connected: false,
            continue_on_error: false,
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
    ContinueOnError(bool),
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
