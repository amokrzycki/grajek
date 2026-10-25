use anyhow::{Context, Result, ensure};
use songbird::input::{
    AudioStream, AudioStreamError, Input, LiveInput, YoutubeDl, metadata::YoutubeDlOutput,
};
use std::{process::Stdio, time::Duration};
use tokio::{
    process::Command,
    time::{Instant, timeout, timeout_at},
};

#[path = "media/availability.rs"]
mod availability;
#[path = "media/range.rs"]
mod range;
pub(crate) use range::SourceControl;

const MAX_TITLE_CHARS: usize = 200;

/// What yt-dlp reported about a track, taken from the same extraction as the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub duration: Option<Duration>,
    pub seekable: bool,
}

impl TrackInfo {
    /// Normalizes untrusted yt-dlp fields; a missing or nonsensical value becomes `None`.
    fn from_ytdlp(title: Option<&str>, seconds: Option<f64>) -> Self {
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

/// A playable stream together with its display metadata.
pub struct Resolved {
    pub input: Input,
    pub info: TrackInfo,
    pub source: Option<SourceControl>,
}

pub const FORMAT: &str = "bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio";

fn extraction_args() -> Vec<&'static str> {
    vec![
        "--ignore-config",
        "--no-playlist",
        "--js-runtimes",
        "deno",
        "--remote-components",
        "ejs:npm",
        "--socket-timeout",
        "15",
        "--retries",
        "0",
        "--extractor-retries",
        "0",
    ]
}

pub fn ytdlp_args() -> Vec<&'static str> {
    let mut args = extraction_args();
    args.extend(["-f", FORMAT, "-j"]);
    args
}

/// Avoid console flashes when a Windows GUI app invokes the extractor tools.
pub(crate) fn tool_command(tool: &str) -> Command {
    #[cfg(windows)]
    let mut command = Command::new(tool);
    #[cfg(not(windows))]
    let command = Command::new(tool);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

pub async fn check_tools() -> Result<()> {
    for tool in ["yt-dlp", "deno"] {
        let output = timeout(
            Duration::from_secs(5),
            tool_command(tool)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await
        .with_context(|| format!("{tool} version check timed out."))?
        .with_context(|| format!("Install {tool} and put it on PATH."))?;
        ensure!(
            output.status.success(),
            "Cannot run {tool}. Check its installation."
        );
        if tool == "deno" {
            let version = String::from_utf8_lossy(&output.stdout);
            ensure!(supported_deno(&version), "Deno 2.3 or newer is required.");
        }
    }
    Ok(())
}

fn supported_deno(output: &str) -> bool {
    let Some(version) = output.split_whitespace().nth(1) else {
        return false;
    };
    let mut parts = version.split('.');
    match (
        parts.next().and_then(|s| s.parse::<u32>().ok()),
        parts.next().and_then(|s| s.parse::<u32>().ok()),
    ) {
        (Some(major), Some(minor)) => major > 2 || (major == 2 && minor >= 3),
        _ => false,
    }
}

/// Extracts display metadata only: no format selection, stream access or decoding.
pub async fn title(url: String) -> Result<Option<String>> {
    let output = tool_command("yt-dlp")
        .args(title_args())
        .arg(url)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("Could not start yt-dlp.")?;
    ensure!(output.status.success(), "yt-dlp title extraction failed");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(TrackInfo::from_ytdlp(
        metadata.get("title").and_then(serde_json::Value::as_str),
        None,
    )
    .title)
}

fn title_args() -> Vec<&'static str> {
    let mut args = extraction_args();
    args.extend([
        "--skip-download",
        "--ignore-no-formats-error",
        "--dump-single-json",
    ]);
    args
}

pub async fn resolve(client: reqwest::Client, url: String) -> Result<Resolved> {
    resolve_format(client, url, FORMAT).await
}

/// Also used by the isolated real-audio spike to exercise the M4A fallback explicitly.
pub(super) async fn resolve_format(
    client: reqwest::Client,
    url: String,
    format: &str,
) -> Result<Resolved> {
    let deadline = Instant::now() + Duration::from_secs(60);
    timeout_at(deadline, async {
        // Songbird's query() does not kill yt-dlp on cancellation; own only that subprocess.
        let output = tool_command("yt-dlp")
            .args(
                ytdlp_args()
                    .into_iter()
                    .map(|arg| if arg == FORMAT { format } else { arg }),
            )
            .arg(&url)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .context("Could not start yt-dlp. Check PATH.")?;
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
    .context("Audio resolution timed out. Check the network and yt-dlp/EJS installation.")?
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

fn extraction_error(stderr: &[u8]) -> String {
    // Classify stderr without exposing signed URLs or authentication diagnostics.
    let error = anyhow::anyhow!(String::from_utf8_lossy(stderr).into_owned())
        .context("yt-dlp extraction failed. Run extraction manually for details.");
    user_error(&error)
}

fn stream_error(error: AudioStreamError) -> AudioStreamError {
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

pub fn user_error(error: &anyhow::Error) -> String {
    let detail = format!("{error:#}");
    let lower = detail.to_lowercase();
    if lower.contains("timed out") {
        error.to_string()
    } else if lower.contains("403") {
        "YouTube denied the audio stream (HTTP 403). Try an accessible video.".into()
    } else if lower.contains("sign in")
        || lower.contains("not a bot")
        || lower.contains("private video")
        || lower.contains("unavailable")
        || lower.contains("age") && lower.contains("restrict")
    {
        "YouTube restricted this video. Try a public video that permits playback.".into()
    } else if lower.contains("ejs") || lower.contains("challenge") || lower.contains("javascript") {
        "YouTube challenge failed. Update yt-dlp, Deno and EJS; check the logs.".into()
    } else if lower.contains("yt-dlp extraction failed") {
        "YouTube extraction failed. Check the logs and update yt-dlp.".into()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Resolved, TrackInfo, check_tools, resolve, supported_deno, user_error};
    use anyhow::{Context, Result, ensure};
    use std::time::Duration;
    use tokio::time::timeout;

    #[test]
    fn title_extraction_is_metadata_only() {
        let args = super::title_args();
        for arg in [
            "--ignore-config",
            "--no-playlist",
            "--skip-download",
            "--ignore-no-formats-error",
            "--dump-single-json",
            "--retries",
        ] {
            assert!(args.contains(&arg));
        }
        assert!(!args.contains(&"-f"));
        assert!(super::ytdlp_args().contains(&super::FORMAT));
    }

    #[test]
    fn checks_minimum_deno_version() {
        for value in ["deno 2.3.0 (stable)", "deno 2.9.7", "deno 3.0.0"] {
            assert!(supported_deno(value));
        }
        for value in ["deno 2.2.9", "deno 1.99.0", "", "deno broken"] {
            assert!(!supported_deno(value));
        }
    }

    #[test]
    fn access_denial_is_not_reported_as_success() {
        assert!(user_error(&anyhow::anyhow!("stream failed HTTP 403")).contains("denied"));
        assert!(
            user_error(&anyhow::anyhow!(
                "yt-dlp extraction failed: Sign in to confirm you’re not a bot"
            ))
            .contains("restricted")
        );
    }

    #[test]
    fn extractor_diagnostics_never_echo_sensitive_stderr() {
        for prefix in ["", "403", "timed out", "Sign in", "EJS challenge"] {
            let stderr = format!(
                "{prefix} https://example.invalid/audio?signature=secret Cookie: private-cookie Authorization: private-token"
            );
            let message = super::extraction_error(stderr.as_bytes());
            for sensitive in [
                "https://",
                "signature",
                "secret",
                "private-cookie",
                "private-token",
            ] {
                assert!(!message.contains(sensitive), "{message}");
            }
        }
        assert!(super::extraction_error(b"HTTP 403").contains("403"));
        assert!(super::extraction_error(b"Sign in to confirm").contains("restricted"));
        assert!(super::extraction_error(b"EJS challenge failed").contains("challenge"));
    }

    #[test]
    fn metadata_is_normalized_and_optional() {
        let info = TrackInfo::from_ytdlp(Some("  Big \n Buck\tBunny  "), Some(596.5));
        assert_eq!(info.title.as_deref(), Some("Big Buck Bunny"));
        assert_eq!(info.duration, Some(Duration::from_secs_f64(596.5)));
        // Live streams and odd extractors: no duration, blank or absent title.
        for seconds in [
            None,
            Some(0.0),
            Some(-3.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(
                TrackInfo::from_ytdlp(Some("   "), seconds),
                TrackInfo::default()
            );
        }
        assert_eq!(TrackInfo::from_ytdlp(None, Some(1e300)).duration, None);
    }

    #[test]
    fn long_titles_are_capped_on_a_character_boundary() {
        let info = TrackInfo::from_ytdlp(Some(&"ż".repeat(1000)), None);
        let title = info.title.unwrap_or_default();
        assert_eq!(title.chars().count(), super::MAX_TITLE_CHARS + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn metadata_comes_from_the_extraction_json() -> Result<()> {
        let json = r#"{"url":"https://example.invalid/a","title":"Song","duration":225.0}"#;
        let output: songbird::input::metadata::YoutubeDlOutput = serde_json::from_str(json)?;
        let info = TrackInfo::from_ytdlp(output.title.as_deref(), output.duration);
        assert_eq!(info.title.as_deref(), Some("Song"));
        assert_eq!(info.duration, Some(Duration::from_secs(225)));
        let output: songbird::input::metadata::YoutubeDlOutput =
            serde_json::from_str(r#"{"url":"https://example.invalid/a"}"#)?;
        assert_eq!(
            TrackInfo::from_ytdlp(output.title.as_deref(), output.duration),
            TrackInfo::default()
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires YouTube network access; uses the openly licensed Big Buck Bunny video"]
    async fn public_video_stream_initializes_with_songbird() -> Result<()> {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        check_tools().await?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()?;
        let url = std::env::var("GRAJEK_TEST_VIDEO_URL")
            .unwrap_or_else(|_| "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into());
        let Resolved { input, info, .. } = resolve(client, url).await?;
        tracing::info!(seekable = info.seekable, "Production stream initialized");
        ensure!(info.title.is_some(), "yt-dlp reported no title");
        ensure!(
            input.is_playable(),
            "Songbird did not initialize the stream"
        );
        timeout(
            Duration::from_secs(15),
            tokio::task::spawn_blocking(move || {
                let mut input = input;
                let parsed = input.parsed_mut().context("stream was not parsed")?;
                loop {
                    let packet = parsed.format.next_packet()?;
                    if packet.track_id() == parsed.track_id {
                        let audio = parsed.decoder.decode(&packet)?;
                        ensure!(audio.frames() > 0, "decoder produced no audio frames");
                        return Ok::<_, anyhow::Error>(());
                    }
                }
            }),
        )
        .await
        .context("audio decode test timed out")???;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "real YouTube/Songbird seek spike; requires network and yt-dlp/Deno"]
    async fn public_video_bidirectional_seek() -> Result<()> {
        use symphonia::core::{
            formats::{SeekMode, SeekTo},
            units::Time,
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()?;
        let Resolved {
            mut input, info, ..
        } = resolve(client, "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into()).await?;
        ensure!(info.seekable, "HTTP range source unavailable");
        let task = tokio::task::spawn_blocking(move || -> Result<()> {
            let parsed = input.parsed_mut().context("not parsed")?;
            ensure!(parsed.supports_backseek, "Symphonia source not seekable");
            for second in [0, 10, 90, 20, 45, 100, 5] {
                let result = parsed.format.seek(
                    SeekMode::Accurate,
                    SeekTo::Time {
                        time: Time::new(second, 0.0),
                        track_id: Some(parsed.track_id),
                    },
                )?;
                parsed.decoder.reset();
                let tb = parsed
                    .decoder
                    .codec_params()
                    .time_base
                    .context("no time base")?;
                let actual = tb.calc_time(result.actual_ts);
                ensure!(
                    (Duration::from_secs(actual.seconds) + Duration::from_secs_f64(actual.frac))
                        .abs_diff(Duration::from_secs(second))
                        < Duration::from_millis(200),
                    "wrong seek position: {actual:?}"
                );
                loop {
                    let packet = parsed.format.next_packet()?;
                    if packet.track_id() == parsed.track_id {
                        ensure!(
                            parsed.decoder.decode(&packet)?.frames() > 0,
                            "no audio after seek"
                        );
                        break;
                    }
                }
                tracing::info!(second, ?actual, "Real YouTube seek decoded");
            }
            Ok(())
        });
        timeout(Duration::from_secs(60), task).await???;
        Ok(())
    }
}
