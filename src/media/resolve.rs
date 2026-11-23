use super::components::{self, Tools, tools};
use super::error::user_error;
use super::range::{self, SourceControl};
use super::track::TrackInfo;
use super::{availability, tool::Tool};
use anyhow::{Context, Result, ensure};
use songbird::input::{
    AudioStream, AudioStreamError, Input, LiveInput, YoutubeDl, metadata::YoutubeDlOutput,
};
use std::{ffi::OsString, time::Duration};
use tokio::time::{Instant, timeout_at};

/// A playable stream together with its display metadata.
pub struct Resolved {
    pub input: Input,
    pub info: TrackInfo,
    pub source: Option<SourceControl>,
}

pub const FORMAT: &str = "bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio";

pub(super) fn ytdlp_args(tools: &Tools, format: &str) -> Vec<OsString> {
    let mut args = tools.extraction_args();
    args.extend(["-f", format, "-j"].map(Into::into));
    args
}

/// Extracts display metadata only: no format selection, stream access or decoding.
pub async fn title(url: String) -> Result<Option<String>> {
    let execution = components::execution::acquire()
        .await
        .map_err(|_| tools().failure())?;
    let tools = execution.tools.clone();
    let mut command = tools.command(Tool::YtDlp);
    command.args(title_args(&tools)).arg(url);
    let output = crate::ytdlp::spawn(command)
        .output()
        .await
        .map_err(|_| tools.failure())?;
    drop(execution);
    ensure!(output.status.success(), "yt-dlp title extraction failed");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(TrackInfo::from_ytdlp(
        metadata.get("title").and_then(serde_json::Value::as_str),
        None,
    )
    .title)
}

pub(super) fn title_args(tools: &Tools) -> Vec<OsString> {
    let mut args = tools.extraction_args();
    args.extend(
        [
            "--skip-download",
            "--ignore-no-formats-error",
            "--dump-single-json",
        ]
        .map(Into::into),
    );
    args
}

pub async fn resolve(client: reqwest::Client, url: String) -> Result<Resolved> {
    resolve_format(client, url, FORMAT).await
}

pub(super) async fn resolve_format(
    client: reqwest::Client,
    url: String,
    format: &str,
) -> Result<Resolved> {
    let execution = components::execution::acquire()
        .await
        .map_err(|_| tools().failure())?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let tools = execution.tools.clone();
    let result = timeout_at(deadline, async {
        // Songbird's query() does not kill yt-dlp on cancellation; own only that subprocess.
        let mut command = tools.command(Tool::YtDlp);
        command.args(ytdlp_args(&tools, format)).arg(&url);
        let output = crate::ytdlp::spawn(command)
            .output()
            .await
            .map_err(|_| tools.failure())?;
        let description = stream_description(&output)?;
        availability::wait_for_stream(&description, deadline).await?;
        let live = description
            .get("is_live")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        let metadata: YoutubeDlOutput = serde_json::from_value(description)
            .context("yt-dlp returned an unsupported stream description.")?;
        let mut info = TrackInfo::from_ytdlp(metadata.title.as_deref(), metadata.duration);
        if live {
            info.duration = None;
        }
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(values) = &metadata.http_headers {
            for (name, value) in values {
                headers.insert(
                    reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                    value.parse()?,
                );
            }
        }
        let ranged =
            if info.duration.is_some() && metadata.protocol.as_deref() != Some("m3u8_native") {
                range::RangeSource::probe(client.clone(), metadata.url.clone(), headers).await
            } else {
                None
            };
        let source = ranged.as_ref().map(|source| source.control.clone());
        if let Some(source) = &source {
            source.limit();
        }
        let mut guard = ResolutionGuard(source.clone());
        let stream = if let Some(source) = ranged {
            info.seekable = true;
            AudioStream {
                input: Box::new(source) as Box<dyn symphonia::core::io::MediaSource>,
            }
        } else {
            YoutubeDl::new(client, url)
                .get_stream(&metadata)
                .await
                .map_err(stream_error)
                .context("Cannot open the audio stream. Check media access and the network.")?
        };
        let mut input = Input::Live(LiveInput::Raw(stream), None)
            .make_playable_async(
                songbird::input::codecs::get_codec_registry(),
                songbird::input::codecs::get_probe(),
            )
            .await
            .context("Cannot initialize or decode the audio stream.")?;
        if let Some(control) = &source {
            control.clear_limit();
            control.enable_seeking();
            input = match input {
                Input::Live(LiveInput::Parsed(mut parsed), compose) => {
                    parsed.supports_backseek = true;
                    parsed.format = Box::new(range::SeekReader {
                        inner: parsed.format,
                        control: control.clone(),
                    });
                    Input::Live(LiveInput::Parsed(parsed), compose)
                }
                other => other,
            };
        }
        guard.0 = None;
        Ok(Resolved {
            input,
            info,
            source,
        })
    })
    .await
    .context("Audio resolution timed out. Check the network and yt-dlp/EJS installation.")?;
    drop(execution);
    result
}

struct ResolutionGuard(Option<SourceControl>);
impl Drop for ResolutionGuard {
    fn drop(&mut self) {
        if let Some(source) = &self.0 {
            source.stop();
        }
    }
}

fn stream_description(output: &std::process::Output) -> Result<serde_json::Value> {
    ensure!(
        output.status.success(),
        "{}",
        extraction_error(&output.stderr)
    );
    if !output.stderr.is_empty() {
        tracing::warn!(
            stage = "yt_dlp",
            "yt-dlp reported diagnostics; run extraction manually for details"
        );
    }
    serde_json::from_slice(&output.stdout).context("yt-dlp returned invalid metadata.")
}

pub(super) fn extraction_error(stderr: &[u8]) -> String {
    // Classify stderr without exposing signed URLs or authentication diagnostics.
    let error = anyhow::anyhow!(String::from_utf8_lossy(stderr).into_owned())
        .context("yt-dlp extraction failed. Run extraction manually for details.");
    user_error(&error)
}

pub(super) fn stream_error(error: AudioStreamError) -> AudioStreamError {
    match error {
        AudioStreamError::Fail(cause) => {
            AudioStreamError::Fail(match cause.downcast::<reqwest::Error>() {
                Ok(error) => Box::new(error.without_url()),
                Err(cause) => cause,
            })
        }
        other => other,
    }
}
