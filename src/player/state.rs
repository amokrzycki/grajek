use std::time::Duration;

use crate::{
    favorites::{self, FavoriteEntry, FavoriteResult},
    history::HistoryEntry,
    media::TrackInfo,
    playlist::ImportStatus,
    preferences::{Preferences, RepeatMode, VOLUME_MAX, clamp_volume},
    queue::{Placement, QueueItem},
    search::{SearchAction, SearchState},
    store,
};

use super::MediaCommand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Initializing,
    Ready,
    Connecting,
    Resolving,
    Playing,
    Paused,
    Stopping,
    Disconnecting,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SeekStatus {
    #[default]
    Ready,
    Pending(Duration),
    Complete(Duration),
    Failed(String),
}

/// Keep End away from natural completion. Duration makes negative/nonfinite intents unrepresentable.
pub fn seek_target(position: Duration, duration: Option<Duration>) -> Option<Duration> {
    duration
        .filter(|duration| !duration.is_zero())
        .map(|duration| position.min(duration.saturating_sub(Duration::from_secs(1))))
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
            Self::Disconnecting => "Leaving voice…",
            Self::Error => "Error",
        }
    }
}

/// Answer to an insert or move request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnqueueResult {
    pub request_id: u64,
    /// The entry now at its requested place. None means the controller rejected or ignored the
    /// request; success remains visible after consumption.
    pub item_id: Option<u64>,
}

/// A snapshot of everything the window shows. The controller owns the truth; the GUI
/// only renders these and sends [`Command`]s and [`VolumeRequest`]s.
#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Playback capabilities, setup activity and tray preference are independent snapshot facts"
)]
pub struct State {
    pub close_to_tray: bool,
    pub setup: crate::discord::setup::Setup,
    pub setup_busy: bool,
    pub confirm_setup: bool,
    /// Last UI transport request accepted by the controller; unrelated snapshots retain it.
    pub transport_ack: u64,
    pub enqueue_result: Option<EnqueueResult>,
    pub phase: Phase,
    pub error: Option<String>,
    /// Monotonic request to show the playback-components prompt; ordinary snapshots retain it.
    pub components_request: u64,
    pub available: bool,
    /// Local queue/history/favorites edits need a running controller, even if the gateway is
    /// down.
    pub controller_ready: bool,
    pub voice_connected: bool,
    pub preferences: Preferences,
    /// Metadata of the current track while it is playing or paused.
    pub track: Option<TrackInfo>,
    /// Playback position, truncated to whole seconds so it changes at most once per second.
    pub position: Duration,
    /// Changes for every newly started track, even when repeating the same URL.
    pub track_id: u64,
    pub seek: SeekStatus,
    /// Increments on each confirmed seek, including repeated seeks to the same timestamp.
    pub seek_serial: u64,
    /// Bot volume in percent, as actually applied.
    pub volume: u8,
    /// The newest [`VolumeRequest::seq`] the controller has handled, applied or not.
    pub volume_seq: u64,
    pub queue: Vec<QueueItem>,
    pub import: ImportStatus,
    pub search: SearchState,
    pub history: Vec<HistoryEntry>,
    /// Newest-first favorites, unique by canonical URL.
    pub favorites: Vec<FavoriteEntry>,
    /// The latest favorites acknowledgement, retained across unrelated snapshots.
    pub favorite_result: Option<FavoriteResult>,
    /// The canonical URL of the active track or the exact request being prepared.
    pub current_url: Option<String>,
    /// A non-fatal remark from startup, such as a recovered history file.
    pub notice: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            setup: crate::discord::setup::Setup::default(),
            setup_busy: false,
            confirm_setup: false,
            transport_ack: 0,
            enqueue_result: None,
            phase: Phase::Initializing,
            error: None,
            components_request: 0,
            available: false,
            controller_ready: false,
            voice_connected: false,
            preferences: Preferences::default(),
            track: None,
            position: Duration::ZERO,
            track_id: 0,
            seek: SeekStatus::Ready,
            seek_serial: 0,
            volume: store::DEFAULT_VOLUME,
            volume_seq: 0,
            queue: Vec::new(),
            import: ImportStatus::Ready,
            search: SearchState::default(),
            history: Vec::new(),
            favorites: Vec::new(),
            favorite_result: None,
            current_url: None,
            notice: None,
        }
    }
}

impl State {
    pub fn can_seek(&self) -> bool {
        self.available
            && matches!(self.phase, Phase::Playing | Phase::Paused)
            && self.track.as_ref().is_some_and(|track| {
                track.seekable && seek_target(Duration::ZERO, track.duration).is_some()
            })
            && !matches!(self.seek, SeekStatus::Failed(_))
    }
    pub fn can_import(&self) -> bool {
        self.controller_ready
            && self.import != ImportStatus::Importing
            && !crate::queue::is_full(self.queue.len())
    }
    pub fn can_leave_voice(&self) -> bool {
        self.controller_ready
            && !matches!(
                self.phase,
                Phase::Initializing | Phase::Stopping | Phase::Disconnecting
            )
            && (self.voice_connected
                || matches!(self.phase, Phase::Connecting | Phase::Resolving)
                || self.available && self.phase == Phase::Error)
    }
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
            && !matches!(
                self.phase,
                Phase::Initializing | Phase::Stopping | Phase::Disconnecting
            )
    }
    /// The transport action's idle label comes from the same authoritative snapshot.
    pub fn next_label(&self) -> &'static str {
        if matches!(self.phase, Phase::Ready | Phase::Error) && self.track.is_none() {
            "Play Queue"
        } else {
            "Next"
        }
    }
    /// Queueing only records a URL, so it works in every phase once Discord is up.
    pub fn can_enqueue(&self) -> bool {
        self.available && self.phase != Phase::Initializing
    }
    /// Queue next also needs a free slot; moving an existing entry never does.
    pub fn can_queue_next(&self) -> bool {
        self.can_enqueue() && !crate::queue::is_full(self.queue.len())
    }
    /// The track to show: only while audio is actually playing or paused.
    pub fn visible_track(&self) -> Option<&TrackInfo> {
        self.track
            .as_ref()
            .filter(|_| matches!(self.phase, Phase::Playing | Phase::Paused))
    }
    /// Whether this snapshot holds a favorite with the given canonical URL.
    pub fn is_favorite(&self, url: &str) -> bool {
        favorites::contains(&self.favorites, url)
    }
    pub fn can_pause(&self) -> bool {
        self.available && self.phase == Phase::Playing
    }
    pub fn can_resume(&self) -> bool {
        self.available && self.phase == Phase::Paused
    }
}

pub enum Command {
    CloseToTray(bool),
    Setup(crate::discord::setup::Request),
    ConfirmSetup(bool),
    /// UI transport requests carry an identity so older snapshots cannot unlock newer input.
    Transport {
        id: u64,
        command: Box<Self>,
    },
    Media(MediaCommand),
    Seek {
        track_id: u64,
        position: Duration,
    },
    /// Plays now, replacing the current track. The pending queue is untouched.
    Play(String),
    EnqueueRequest {
        id: u64,
        value: String,
        placement: Placement,
    },
    /// Makes an existing pending entry the next one; the answer is an [`EnqueueResult`].
    MoveToNext {
        request_id: u64,
        id: u64,
    },
    ImportPlaylist(String),
    CancelImport,
    Search {
        id: u64,
        query: String,
    },
    CancelSearch {
        id: u64,
    },
    SearchAction {
        search_id: u64,
        video_id: String,
        action: SearchAction,
    },
    /// Desired-state membership for a non-search Save/Remove. `url` is canonicalized.
    SetFavorite {
        id: u64,
        url: String,
        saved: bool,
    },
    Next,
    Pause,
    Resume,
    Stop,
    LeaveVoice,
    Dequeue(u64),
    Shift {
        id: u64,
        up: bool,
    },
    ClearQueue,
    ForgetHistory(String),
    ClearHistory,
    ContinueOnError(bool),
    RememberQueue(bool),
    Repeat(RepeatMode),
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
    f32::from(clamp_volume(percent)) / f32::from(VOLUME_MAX)
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
        .then(|| value.round().clamp(0.0, f32::from(VOLUME_MAX)) as u8)
}
