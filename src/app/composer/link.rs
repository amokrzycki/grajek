//! Local recognition of the pasted text: what it is, never what to do with it.

use crate::youtube::{playlist_url, video_url};

/// What the pasted text is, judged locally. Recognition never starts playback or import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum LinkKind {
    Empty,
    Video,
    /// A watch link that also names a playlist: Enter and Add still use the single video.
    VideoInPlaylist,
    Playlist,
    Other,
    Query,
}

impl LinkKind {
    pub(in crate::app) fn of(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            Self::Empty
        } else if text.contains("list=") && playlist_url(text).is_ok() {
            if video_url(text).is_ok() {
                Self::VideoInPlaylist
            } else {
                Self::Playlist
            }
        } else if video_url(text).is_ok() {
            Self::Video
        } else if url_shaped(text) {
            Self::Other
        } else {
            Self::Query
        }
    }

    pub(super) fn is_playlist(self) -> bool {
        matches!(self, Self::Playlist | Self::VideoInPlaylist)
    }

    pub(super) fn message(self) -> Option<String> {
        match self {
            Self::Empty | Self::Other | Self::Query => None,
            Self::Video => Some("Video link recognized.".into()),
            Self::VideoInPlaylist => Some(
                "Video in a playlist · Play now, Add to queue and Queue next use this video only."
                    .into(),
            ),
            Self::Playlist => Some(format!(
                "Playlist link · Import playlist adds up to {} tracks.",
                crate::playlist::ENTRY_LIMIT
            )),
        }
    }
}

fn url_shaped(text: &str) -> bool {
    let lower = text.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    lower.contains("://")
        || first.starts_with("www.")
        || first.starts_with("youtube.")
        || first.starts_with("youtu.be")
        || first.starts_with("www.youtube.")
        || first.starts_with("m.youtube.")
        || first.starts_with("music.youtube.")
        || first
            .split_once('/')
            .is_some_and(|(host, _)| host.contains('.'))
        || ["http:", "https:", "file:", "ftp:", "data:", "javascript:"]
            .iter()
            .any(|scheme| first.starts_with(scheme))
        || text.split_once(':').is_some_and(|(scheme, rest)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
                && rest.chars().next().is_some_and(|ch| !ch.is_whitespace())
        })
}
