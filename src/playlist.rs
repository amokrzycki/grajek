//! Bounded, metadata-only `YouTube` playlist extraction. No voice or audio preparation.

use std::{
    fmt::{self, Write as _},
    time::Duration,
};
use tokio::time::timeout;

use crate::youtube::{playlist_url, safe_title, valid_video_id, watch_url};

pub(crate) const ENTRY_LIMIT: usize = crate::queue::QUEUE_LIMIT;
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playlist {
    pub entries: Vec<Entry>,
    pub unavailable: usize,
    /// The bounded scan may have omitted further entries; never an exact skipped count.
    pub limited: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidUrl,
    Tool,
    MissingComponents,
    Unavailable,
    Timeout,
    OutputLimit,
    Malformed,
    Empty,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidUrl => "Use a valid YouTube playlist URL.",
            Self::Tool => "Could not run yt-dlp. Check its installation and try again.",
            Self::MissingComponents => "Playback components are missing or damaged. Open Settings to download or repair them.",
            Self::Unavailable => {
                "Playlist unavailable. Check the public link and network, then try again."
            }
            Self::Timeout => "Playlist import timed out. Check the network and try again.",
            Self::OutputLimit => {
                "Playlist metadata exceeded the safety limit. Try a smaller playlist."
            }
            Self::Malformed => {
                "YouTube returned invalid playlist metadata. Update yt-dlp and try again."
            }
            Self::Empty => "No usable entries found in this playlist.",
        })
    }
}
impl std::error::Error for Error {}

impl From<crate::ytdlp::Failure> for Error {
    fn from(failure: crate::ytdlp::Failure) -> Self {
        match failure {
            crate::ytdlp::Failure::Tool => Self::Tool,
            crate::ytdlp::Failure::Unavailable => Self::Unavailable,
            crate::ytdlp::Failure::OutputLimit => Self::OutputLimit,
        }
    }
}

/// Import feedback is independent of playback phase and errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ImportStatus {
    #[default]
    Ready,
    Importing,
    Cancelled,
    Finished {
        added: usize,
        unavailable: usize,
        omitted: usize,
        limited: bool,
    },
    Failed(String),
}

impl ImportStatus {
    pub fn message(&self) -> String {
        match self {
            Self::Ready => format!("Playlist import · up to {ENTRY_LIMIT} pending tracks"),
            Self::Importing => "Importing playlist…".into(),
            Self::Cancelled => "Import cancelled.".into(),
            Self::Failed(message) => message.clone(),
            Self::Finished {
                added,
                unavailable,
                omitted,
                limited,
            } => {
                let mut message = format!(
                    "Imported {added} {}.",
                    if *added == 1 { "track" } else { "tracks" }
                );
                if *unavailable > 0 {
                    let _ = write!(message, " {unavailable} entries unavailable.");
                }
                if *omitted > 0 {
                    let _ = write!(
                        message,
                        " {omitted} did not fit; {}.",
                        crate::queue::limit_note()
                    );
                }
                if *limited {
                    let _ = write!(
                        message,
                        " First {ENTRY_LIMIT} playlist entries checked; more may remain."
                    );
                }
                message
            }
        }
    }
}

pub async fn extract(url: String) -> Result<Playlist, Error> {
    let url = playlist_url(&url).map_err(|_| Error::InvalidUrl)?;
    let (execution, mut command) = crate::media::ytdlp_command()
        .await
        .map_err(|_| Error::MissingComponents)?;
    command
        .args(crate::ytdlp::flat_args(ENTRY_LIMIT, true))
        .arg(url);
    let child = crate::ytdlp::spawn(command)
        .spawn()
        .map_err(|_| Error::MissingComponents)?;
    let bytes = collect(child, TIMEOUT).await?;
    drop(execution);
    parse(&bytes)
}

async fn collect(mut child: tokio::process::Child, deadline: Duration) -> Result<Vec<u8>, Error> {
    let result = timeout(
        deadline,
        crate::ytdlp::read_output(&mut child, OUTPUT_LIMIT, DIAGNOSTIC_LIMIT),
    )
    .await
    .map_err(|_| Error::Timeout)
    .and_then(|result| result.map_err(Error::from));
    if result.is_err() {
        // Explicit error paths reap before returning; dropping a cancelled future kills the child.
        let _ = child.kill().await;
    }
    result
}

fn parse(bytes: &[u8]) -> Result<Playlist, Error> {
    let root: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
    let items = crate::ytdlp::playlist_entries(&root).ok_or(Error::Malformed)?;
    let limited = items.len() >= ENTRY_LIMIT;
    let mut entries = Vec::new();
    let mut unavailable = 0;
    for item in items.iter().take(ENTRY_LIMIT) {
        let Some(id) = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| valid_video_id(id))
        else {
            unavailable += 1;
            continue;
        };
        if crate::ytdlp::unavailable_entry(item) {
            unavailable += 1;
            continue;
        }
        entries.push(Entry {
            url: watch_url(id),
            title: item
                .get("title")
                .and_then(serde_json::Value::as_str)
                .and_then(safe_title),
        });
    }
    if entries.is_empty() {
        return Err(Error::Empty);
    }
    Ok(Playlist {
        entries,
        unavailable,
        limited,
    })
}

#[cfg(test)]
mod tests;
