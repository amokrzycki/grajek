use super::components::tools;
use super::error::user_error;
use super::resolve::{FORMAT, Resolved, extraction_error, resolve, title_args, ytdlp_args};
use super::tool::{check_tools, supported_deno};
use super::track::TrackInfo;
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use tokio::time::timeout;

#[test]
fn title_extraction_is_metadata_only() {
    let args = title_args(&tools());
    for arg in [
        "--ignore-config",
        "--no-playlist",
        "--skip-download",
        "--ignore-no-formats-error",
        "--dump-single-json",
        "--retries",
    ] {
        assert!(args.iter().any(|value| value == arg));
    }
    assert!(!args.iter().any(|value| value == "-f"));
    assert!(
        ytdlp_args(&tools(), FORMAT)
            .iter()
            .any(|value| value == FORMAT)
    );
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
        let message = extraction_error(stderr.as_bytes());
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
    assert!(extraction_error(b"HTTP 403").contains("403"));
    assert!(extraction_error(b"Sign in to confirm").contains("restricted"));
    assert!(extraction_error(b"EJS challenge failed").contains("challenge"));
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
    assert_eq!(title.chars().count(), crate::youtube::MAX_TITLE_CHARS + 1);
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
