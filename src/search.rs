use std::{fmt, future::Future, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::Child,
    sync::watch,
    time::{Instant, timeout},
};

use crate::config::valid_video_id;

const ENTRY_LIMIT: usize = 5;
const STDOUT_LIMIT: usize = 256 * 1024;
const STDERR_LIMIT: usize = 64 * 1024;
const ACQUIRE_LIMIT: Duration = Duration::from_secs(5);
pub const TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultItem {
    pub url: String,
    pub title: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<Duration>,
    pub live: bool,
}

impl ResultItem {
    pub fn id(&self) -> &str {
        self.url.rsplit_once("v=").map_or("", |(_, id)| id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Results {
    pub items: Vec<ResultItem>,
    pub rejected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidQuery,
    Tool,
    MissingComponents,
    Unavailable,
    Timeout,
    OutputLimit,
    Malformed,
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidQuery => "Enter a search query of at most 200 characters.",
            Self::Tool => {
                "Could not run YouTube search. Check the playback components and try again."
            }
            Self::MissingComponents => {
                "Playback components are missing, damaged, or busy. Open Settings to repair them."
            }
            Self::Unavailable => "YouTube search is unavailable. Check the network and try again.",
            Self::Timeout => "YouTube search timed out. Check the network and try again.",
            Self::OutputLimit => "Search metadata exceeded the safety limit. Try a shorter query.",
            Self::Malformed => {
                "YouTube returned invalid search metadata. Update yt-dlp and try again."
            }
            Self::Cancelled => "YouTube search was cancelled.",
        })
    }
}

impl std::error::Error for Error {}

pub fn query(input: &str) -> Result<String, Error> {
    normalize(input, 200).ok_or(Error::InvalidQuery)
}

fn normalize(input: &str, max: usize) -> Option<String> {
    if input
        .chars()
        .any(|ch| ch.is_control() && !ch.is_whitespace())
    {
        return None;
    }
    let value = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() || value.chars().count() > max {
        None
    } else {
        Some(value)
    }
}

fn args() -> [&'static str; 17] {
    [
        "--ignore-config",
        "--no-plugin-dirs",
        "--flat-playlist",
        "--dump-single-json",
        "--playlist-end",
        "5",
        "--lazy-playlist",
        "--skip-download",
        "--socket-timeout",
        "15",
        "--retries",
        "0",
        "--extractor-retries",
        "0",
        "--no-warnings",
        "--no-progress",
        "--",
    ]
}

pub async fn extract(input: String, mut cancel: watch::Receiver<bool>) -> Result<Results, Error> {
    let query = query(&input)?;
    let started = Instant::now();
    let budget = TIMEOUT;
    if *cancel.borrow() {
        return Err(Error::Cancelled);
    }
    let acquire_for = ACQUIRE_LIMIT.min(budget.saturating_sub(started.elapsed()));
    let execution = acquire_bounded(
        &mut cancel,
        acquire_for,
        crate::media::components::execution::acquire(),
    )
    .await?;
    if started.elapsed() >= budget {
        return Err(Error::Timeout);
    }
    let mut command = execution.tools.command(crate::media::Tool::YtDlp);
    command
        .args(args())
        .arg(format!("ytsearch5:{query}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().map_err(|_| Error::MissingComponents)?;
    let bytes = collect(
        child,
        cancel,
        budget.saturating_sub(started.elapsed()),
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    drop(execution);
    parse(&bytes)
}

async fn acquire_bounded<T>(
    cancel: &mut watch::Receiver<bool>,
    limit: Duration,
    acquire: impl Future<Output = anyhow::Result<T>>,
) -> Result<T, Error> {
    if *cancel.borrow() {
        return Err(Error::Cancelled);
    }
    tokio::select! {
        biased;
        changed = cancel.changed() => {
            let _ = changed;
            Err(Error::Cancelled)
        }
        result = timeout(limit, acquire) => {
            match result {
                Err(_) | Ok(Err(_)) => Err(Error::MissingComponents),
                Ok(Ok(value)) => Ok(value),
            }
        }
    }
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

async fn collect(
    mut child: Child,
    mut cancel: watch::Receiver<bool>,
    deadline: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Vec<u8>, Error> {
    if *cancel.borrow() {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(Error::Cancelled);
    }
    let stdout = child.stdout.take().ok_or(Error::Tool)?;
    let stderr = child.stderr.take().ok_or(Error::Tool)?;
    let result = {
        let operation = async {
            let (output, _diagnostics, status) = tokio::try_join!(
                read_bounded(stdout, stdout_limit),
                read_bounded(stderr, stderr_limit),
                async { child.wait().await.map_err(|_| Error::Tool) },
            )?;
            if !status.success() {
                return Err(Error::Unavailable);
            }
            Ok(output)
        };
        tokio::pin!(operation);
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                let _ = changed;
                Err(Error::Cancelled)
            }
            result = timeout(deadline, &mut operation) => result.unwrap_or(Err(Error::Timeout)),
        }
    };
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

fn clean_text(value: &serde_json::Value, max: usize) -> Option<String> {
    let text = value.as_str()?;
    let safe: String = text
        .chars()
        .filter(|ch| !ch.is_control() || ch.is_whitespace())
        .collect();
    normalize(&safe, max)
}

fn parse(bytes: &[u8]) -> Result<Results, Error> {
    let root: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
    if root.get("_type").and_then(serde_json::Value::as_str) != Some("playlist") {
        return Err(Error::Malformed);
    }
    let entries = root
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or(Error::Malformed)?;
    if entries.len() > ENTRY_LIMIT {
        return Err(Error::Malformed);
    }
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rejected = 0;
    for entry in entries {
        let Some(item) = parse_item(entry) else {
            rejected += 1;
            continue;
        };
        if !seen.insert(item.id().to_owned()) {
            rejected += 1;
            continue;
        }
        items.push(item);
    }
    Ok(Results { items, rejected })
}

fn parse_item(entry: &serde_json::Value) -> Option<ResultItem> {
    let id = entry
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_video_id(id))?;
    if entry
        .get("_type")
        .is_some_and(|value| !matches!(value.as_str(), Some("url" | "url_transparent" | "video")))
        || entry
            .get("ie_key")
            .is_some_and(|value| value.as_str() != Some("Youtube"))
        || entry
            .get("extractor_key")
            .is_some_and(|value| value.as_str() != Some("Youtube"))
        || entry
            .get("extractor")
            .is_some_and(|value| !matches!(value.as_str(), Some("youtube" | "Youtube")))
        || entry
            .get("availability")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| {
                matches!(
                    value,
                    "private" | "premium_only" | "subscriber_only" | "needs_auth" | "unavailable"
                )
            })
        || entry
            .get("title")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| {
                matches!(
                    value,
                    "[Deleted video]" | "[Private video]" | "[Unavailable video]"
                )
            })
    {
        return None;
    }
    for key in ["url", "webpage_url"] {
        if let Some(value) = entry.get(key) {
            let url = value.as_str()?;
            match canonical_id(url) {
                Ok(Some(url_id)) if url_id != id => return None,
                Err(()) => return None,
                _ => {}
            }
        }
    }
    let live = entry.get("is_live").and_then(serde_json::Value::as_bool) == Some(true)
        || entry.get("live_status").and_then(serde_json::Value::as_str) == Some("is_live");
    let duration = if live {
        None
    } else {
        entry
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .and_then(|seconds| {
                Duration::try_from_secs_f64(seconds)
                    .ok()
                    .filter(|duration| !duration.is_zero())
            })
    };
    Some(ResultItem {
        url: format!("https://www.youtube.com/watch?v={id}"),
        title: entry.get("title").and_then(|v| clean_text(v, 4096)),
        channel: entry.get("channel").and_then(|v| clean_text(v, 512)),
        duration,
        live,
    })
}

fn canonical_id(value: &str) -> Result<Option<String>, ()> {
    let Ok(url) = url::Url::parse(value) else {
        return Ok(None);
    };
    let Some(host) = url.host_str() else {
        return Ok(None);
    };
    if matches!(host, "youtu.be" | "www.youtu.be") {
        let id = url
            .path_segments()
            .and_then(|mut paths| paths.next())
            .ok_or(())?;
        return valid_video_id(id).then(|| Some(id.to_owned())).ok_or(());
    }
    if !matches!(
        host,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com"
    ) {
        return Ok(None);
    }
    let id = match url.path() {
        "/watch" => url
            .query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned()),
        path if path.starts_with("/shorts/") || path.starts_with("/embed/") => {
            path.split('/').nth(2).map(str::to_owned)
        }
        _ => return Ok(None),
    }
    .ok_or(())?;
    valid_video_id(&id).then_some(Some(id)).ok_or(())
}

#[cfg(test)]
mod tests;
