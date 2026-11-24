use std::{fmt, future::Future, time::Duration};

use tokio::{
    process::Child,
    sync::watch,
    time::{Instant, timeout},
};

use crate::{queue::Placement, youtube::valid_video_id};

pub(crate) const ENTRY_LIMIT: usize = 5;
pub(crate) const QUERY_LIMIT: usize = 200;
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
        crate::youtube::video_id(&self.url)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Results {
    pub items: Vec<ResultItem>,
    pub rejected: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchState {
    pub id: u64,
    pub query: String,
    pub phase: SearchPhase,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SearchPhase {
    #[default]
    Idle,
    Loading,
    Results(Results),
    Empty {
        rejected: usize,
    },
    Failed(Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    Play,
    Enqueue {
        id: u64,
        placement: Placement,
    },
    /// Desired-state membership for the selected search result; the controller routes it
    /// before consuming Results.
    Favorite {
        id: u64,
        saved: bool,
    },
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
        match self {
            Self::InvalidQuery => {
                write!(
                    f,
                    "Enter a search query of at most {QUERY_LIMIT} characters."
                )
            }
            Self::Tool => f.write_str(
                "Could not run YouTube search. Check the playback components and try again.",
            ),
            Self::MissingComponents => f.write_str(
                "Playback components are missing, damaged, or busy. Open Settings to repair them.",
            ),
            Self::Unavailable => {
                f.write_str("YouTube search is unavailable. Check the network and try again.")
            }
            Self::Timeout => {
                f.write_str("YouTube search timed out. Check the network and try again.")
            }
            Self::OutputLimit => {
                f.write_str("Search metadata exceeded the safety limit. Try a shorter query.")
            }
            Self::Malformed => f.write_str(
                "YouTube returned invalid search metadata. Update yt-dlp and try again.",
            ),
            Self::Cancelled => f.write_str("YouTube search was cancelled."),
        }
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

pub fn query(input: &str) -> Result<String, Error> {
    normalize(input, QUERY_LIMIT).ok_or(Error::InvalidQuery)
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

pub async fn extract(input: String, mut cancel: watch::Receiver<bool>) -> Result<Results, Error> {
    let query = query(&input)?;
    let started = Instant::now();
    let budget = TIMEOUT;
    if *cancel.borrow() {
        return Err(Error::Cancelled);
    }
    let acquire_for = ACQUIRE_LIMIT.min(budget.saturating_sub(started.elapsed()));
    let (execution, mut command) =
        acquire_bounded(&mut cancel, acquire_for, crate::media::ytdlp_command()).await?;
    if started.elapsed() >= budget {
        return Err(Error::Timeout);
    }
    command
        .args(crate::ytdlp::flat_args(ENTRY_LIMIT, false))
        .arg(format!("ytsearch{ENTRY_LIMIT}:{query}"));
    let child = crate::ytdlp::spawn(command)
        .spawn()
        .map_err(|_| Error::MissingComponents)?;
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
    let result = {
        let operation = crate::ytdlp::read_output(&mut child, stdout_limit, stderr_limit);
        tokio::pin!(operation);
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                let _ = changed;
                Err(Error::Cancelled)
            }
            result = timeout(deadline, &mut operation) => match result {
                Ok(result) => result.map_err(Error::from),
                Err(_) => Err(Error::Timeout),
            },
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
    let entries = crate::ytdlp::playlist_entries(&root).ok_or(Error::Malformed)?;
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
    if crate::ytdlp::unavailable_entry(entry)
        || entry
            .get("extractor_key")
            .is_some_and(|value| value.as_str() != Some("Youtube"))
        || entry
            .get("extractor")
            .is_some_and(|value| !matches!(value.as_str(), Some("youtube" | "Youtube")))
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
        url: crate::youtube::watch_url(id),
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
    if crate::youtube::is_short_host(host) {
        let id = url
            .path_segments()
            .and_then(|mut paths| paths.next())
            .ok_or(())?;
        return valid_video_id(id).then(|| Some(id.to_owned())).ok_or(());
    }
    if !crate::youtube::is_watch_host(host) {
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
