use anyhow::{Context, Result, ensure};
use songbird::input::{AudioStream, Input, LiveInput, YoutubeDl, metadata::YoutubeDlOutput};
use std::{process::Stdio, time::Duration};
use tokio::{process::Command, time::timeout};

pub const FORMAT: &str = "bestaudio[acodec=opus][ext=webm]/bestaudio[ext=m4a]/bestaudio";

pub fn ytdlp_args() -> Vec<&'static str> {
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
        "-f",
        FORMAT,
        "-j",
    ]
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

pub async fn resolve(client: reqwest::Client, url: String) -> Result<Input> {
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
            tracing::warn!(diagnostic = %String::from_utf8_lossy(&output.stderr).trim(), "yt-dlp diagnostic");
        }
        let metadata: YoutubeDlOutput = serde_json::from_slice(&output.stdout)
            .context("yt-dlp returned an unsupported stream description.")?;
        let loader = YoutubeDl::new(client, url);
        let stream: AudioStream<_> = loader
            .get_stream(&metadata)
            .await
            .context("Cannot open the audio stream. Check media access and the network.")?;
        Input::Live(LiveInput::Raw(stream), None)
            .make_playable_async(
                songbird::input::codecs::get_codec_registry(),
                songbird::input::codecs::get_probe(),
            )
            .await
            .context("Cannot initialize or decode the audio stream.")
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
    use super::{check_tools, resolve, supported_deno, user_error};
    use anyhow::{Context, Result, ensure};
    use std::time::Duration;
    use tokio::time::timeout;

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

    #[tokio::test]
    #[ignore = "requires YouTube network access; uses the openly licensed Big Buck Bunny video"]
    async fn public_video_stream_initializes_with_songbird() -> Result<()> {
        check_tools().await?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()?;
        let input = resolve(client, "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into()).await?;
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
