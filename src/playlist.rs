//! Bounded, metadata-only `YouTube` playlist extraction. No voice or audio preparation.

use std::{fmt, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::Child,
    time::timeout,
};

use crate::{
    config::{playlist_url, valid_video_id},
    store::safe_title,
};

const ENTRY_LIMIT: usize = 100;
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

fn args() -> [&'static str; 18] {
    [
        "--ignore-config",
        "--no-plugin-dirs",
        "--flat-playlist",
        "--dump-single-json",
        "--playlist-end",
        "100",
        "--lazy-playlist",
        "--skip-download",
        "--yes-playlist",
        "--socket-timeout",
        "15",
        "--retries",
        "0",
        "--extractor-retries",
        "0",
        "--no-warnings",
        "--no-progress",
        "--", // End options before the canonical URL.
    ]
}

pub async fn extract(url: String) -> Result<Playlist, Error> {
    let url = playlist_url(&url).map_err(|_| Error::InvalidUrl)?;
    let child = crate::media::tool_command("yt-dlp")
        .args(args())
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| Error::Tool)?;
    let bytes = collect(child, TIMEOUT).await?;
    parse(&bytes)
}

async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Error::Tool)?;
    if bytes.len() > limit {
        return Err(Error::OutputLimit);
    }
    Ok(bytes)
}

async fn collect(mut child: Child, deadline: Duration) -> Result<Vec<u8>, Error> {
    let stdout = child.stdout.take().ok_or(Error::Tool)?;
    let stderr = child.stderr.take().ok_or(Error::Tool)?;
    let result = timeout(deadline, async {
        let (output, _diagnostics, status) = tokio::try_join!(
            read_bounded(stdout, OUTPUT_LIMIT),
            read_bounded(stderr, DIAGNOSTIC_LIMIT),
            async { child.wait().await.map_err(|_| Error::Tool) },
        )?;
        if !status.success() {
            return Err(Error::Unavailable);
        }
        Ok(output)
    })
    .await
    .unwrap_or(Err(Error::Timeout));
    if result.is_err() {
        // Explicit error paths reap before returning; dropping a cancelled future kills the child.
        let _ = child.kill().await;
    }
    result
}

fn parse(bytes: &[u8]) -> Result<Playlist, Error> {
    let root: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
    if root.get("_type").and_then(serde_json::Value::as_str) != Some("playlist") {
        return Err(Error::Malformed);
    }
    let items = root
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or(Error::Malformed)?;
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
        let kind = item.get("_type");
        let title = item.get("title").and_then(serde_json::Value::as_str);
        let availability = item.get("availability").and_then(serde_json::Value::as_str);
        if kind
            .is_some_and(|kind| !matches!(kind.as_str(), Some("url" | "url_transparent" | "video")))
            || matches!(
                availability,
                Some("private" | "premium_only" | "subscriber_only" | "needs_auth" | "unavailable")
            )
            || matches!(
                title,
                Some("[Deleted video]" | "[Private video]" | "[Unavailable video]")
            )
            || item
                .get("ie_key")
                .is_some_and(|key| key.as_str() != Some("Youtube"))
        {
            unavailable += 1;
            continue;
        }
        entries.push(Entry {
            url: format!("https://www.youtube.com/watch?v={id}"),
            title: title.and_then(safe_title),
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
