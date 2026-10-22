use anyhow::{Context, Result, ensure};
use songbird::input::{AudioStream, Input, LiveInput, YoutubeDl, metadata::YoutubeDlOutput};
use std::{process::Stdio, time::Duration};
use tokio::{process::Command, time::timeout};

const MAX_TITLE_CHARS: usize = 200;

/// What yt-dlp reported about a track, taken from the same extraction as the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub duration: Option<Duration>,
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
        Self { title, duration }
    }
}

/// A playable stream together with its display metadata.
pub struct Resolved {
    pub input: Input,
    pub info: TrackInfo,
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

pub async fn check_tools() -> Result<()> {
    for tool in ["yt-dlp", "deno"] {
        let output = timeout(
            Duration::from_secs(5),
            Command::new(tool)
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
    let output = Command::new("yt-dlp")
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
    timeout(Duration::from_secs(60), async {
        // Songbird's query() does not kill yt-dlp on cancellation; own only that subprocess.
        let output = Command::new("yt-dlp")
            .args(ytdlp_args())
            .arg(&url)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .context("Could not start yt-dlp. Check PATH.")?;
        ensure!(
            output.status.success(),
            "yt-dlp extraction failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        if !output.stderr.is_empty() {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(diagnostic = %diagnostic.trim(), "yt-dlp diagnostic");
        }
        let metadata: YoutubeDlOutput = serde_json::from_slice(&output.stdout)
            .context("yt-dlp returned an unsupported stream description.")?;
        let info = TrackInfo::from_ytdlp(metadata.title.as_deref(), metadata.duration);
        let loader = YoutubeDl::new(client, url);
        let stream: AudioStream<_> = loader
            .get_stream(&metadata)
            .await
            .context("Cannot open the audio stream. Check media access and the network.")?;
        let input = Input::Live(LiveInput::Raw(stream), None)
            .make_playable_async(
                songbird::input::codecs::get_codec_registry(),
                songbird::input::codecs::get_probe(),
            )
            .await
            .context("Cannot initialize or decode the audio stream.")?;
        Ok(Resolved { input, info })
    })
    .await
    .context("Audio resolution timed out. Check the network and yt-dlp/EJS installation.")?
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
        check_tools().await?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()?;
        let Resolved { input, info } =
            resolve(client, "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into()).await?;
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
}
