use crate::youtube::MAX_TITLE_CHARS;
use std::time::Duration;

/// What yt-dlp reported about a track, taken from the same extraction as the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub duration: Option<Duration>,
    pub seekable: bool,
}

impl TrackInfo {
    /// Normalizes untrusted yt-dlp fields; a missing or nonsensical value becomes `None`.
    pub(super) fn from_ytdlp(title: Option<&str>, seconds: Option<f64>) -> Self {
        let title = title
            .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|title| !title.is_empty())
            .map(|title| {
                if title.chars().count() > MAX_TITLE_CHARS {
                    let cut: String = title.chars().take(MAX_TITLE_CHARS).collect();
                    format!("{}…", cut.trim_end())
                } else {
                    title
                }
            });
        let duration = seconds
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .filter(|duration| !duration.is_zero());
        Self {
            title,
            duration,
            seekable: false,
        }
    }
}
