//! Real YouTube pipeline + Songbird's offline mixer harness, enabled only in a temporary copy.
#[allow(
    dead_code,
    reason = "the spike imports the complete production media module"
)]
#[path = "../src/media.rs"]
mod media;
#[allow(
    dead_code,
    reason = "the local diagnostic imports the production range source"
)]
#[path = "../src/media/range.rs"]
mod range;

use anyhow::{Result, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::{sleep, timeout};

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--local") {
        return local_ranges().await;
    }
    media::check_tools().await?;
    for (format, codec) in [
        (media::FORMAT, symphonia::core::codecs::CODEC_TYPE_OPUS),
        (
            "bestaudio[ext=m4a]",
            symphonia::core::codecs::CODEC_TYPE_AAC,
        ),
    ] {
        let mut resolved = media::resolve_format(
            reqwest::Client::new(),
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into(),
            format,
        )
        .await?;
        ensure!(resolved.info.seekable, "HTTP ranges unavailable");
        ensure!(
            resolved
                .input
                .parsed_mut()
                .is_some_and(|parsed| parsed.decoder.codec_params().codec == codec),
            "unexpected selected codec"
        );
        let (ticks, config) = songbird::Config::test_cfg(true);
        let mut driver = songbird::Driver::new(config);
        let audio = Arc::new(AtomicUsize::new(0));
        let observed = audio.clone();
        let ticker = tokio::spawn(async move {
            loop {
                ticks.tick(1);
                if let songbird::driver::OutputPacket::Raw(packet) = ticks.recv_async().await {
                    if packet.is_mixed_with_nonzero_signal() {
                        observed.fetch_add(1, Ordering::SeqCst);
                    }
                }
                sleep(Duration::from_millis(1)).await;
            }
        });
        let handle = driver.play_only(songbird::tracks::Track::from(resolved.input).volume(0.4));
        let result = async {
            timeout(Duration::from_secs(5), handle.make_playable_async()).await??;
            let initial = timeout(Duration::from_secs(2), handle.get_info()).await??;
            ensure!(initial.position < Duration::from_secs(1), "did not start at zero");
            println!("{format}: start {:?}", initial.position);
            for (second, paused) in [(10, false), (90, false), (20, false), (45, true), (100, false), (5, false)] {
                if paused { handle.pause()?; } else { handle.play()?; }
                let target = Duration::from_secs(second);
                let actual = timeout(Duration::from_secs(10), handle.seek_async(target)).await??;
                let state = timeout(Duration::from_secs(2), handle.get_info()).await??;
                ensure!(actual.abs_diff(target) < Duration::from_millis(200), "wrong seek callback position");
                ensure!(state.position.abs_diff(actual) < Duration::from_secs(1), "wrong get_info position");
                ensure!(matches!((&state.playing, paused), (songbird::tracks::PlayMode::Pause, true) | (songbird::tracks::PlayMode::Play, false)), "seek changed play mode");
                ensure!((state.volume - 0.4).abs() < f32::EPSILON, "seek changed volume");
                let before = audio.load(Ordering::SeqCst);
                sleep(Duration::from_millis(40)).await;
                let after = timeout(Duration::from_secs(2), handle.get_info()).await??;
                if paused { ensure!(after.position == state.position, "paused seek advanced"); }
                else { ensure!(audio.load(Ordering::SeqCst) > before, "no mixed audio after seek"); }
                println!("  target={second}s actual={actual:?} reported={:?} paused={paused} volume={} mixed_frames={}", state.position, state.volume, audio.load(Ordering::SeqCst));
            }
            Ok::<_, anyhow::Error>(())
        }.await;
        if let Some(source) = resolved.source {
            source.stop();
        }
        driver.stop();
        ticker.abort();
        let _ = ticker.await;
        result?;
    }
    Ok(())
}

/// Real mixer regression for Songbird's packet-error/End behavior and blocked info callbacks.
async fn local_ranges() -> Result<()> {
    use songbird::{Event, EventContext, EventHandler, TrackEvent, tracks::PlayMode};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{Notify, watch},
    };
    struct End(watch::Sender<Option<PlayMode>>);
    #[serenity::async_trait]
    impl EventHandler for End {
        async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
            if let EventContext::Track(tracks) = context {
                if let Some((state, _)) = tracks.first() {
                    let _ = self.0.send(Some(state.playing.clone()));
                }
            }
            Some(Event::Cancel)
        }
    }
    let mut metadata = serde_json::to_vec(&serde_json::json!({
        "dca":{"version":1,"tool":{"name":"grajek-spike","version":"1"}},
        "opus":{"mode":"audio","sample_rate":48000,"frame_size":960,"vbr":false,"channels":2}
    }))?;
    metadata.resize(262_000, b' ');
    let mut bytes = b"DCA1".to_vec();
    bytes.extend_from_slice(&u32::try_from(metadata.len())?.to_le_bytes());
    bytes.extend_from_slice(&metadata);
    for _ in 0..20_000 {
        bytes.extend_from_slice(&[3, 0, 0xf8, 0xff, 0xfe]); // Valid 20 ms Opus silence packet.
    }
    for fail in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/audio", listener.local_addr()?);
        let requested = Arc::new(Notify::new());
        let observed = requested.clone();
        let body = bytes.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).to_lowercase();
                let interval = request
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .unwrap();
                let (start, end) = interval.split_once('-').unwrap();
                let start: usize = start.parse().unwrap();
                let end = end.parse::<usize>().unwrap().min(body.len() - 1);
                if start != 0 {
                    observed.notify_one();
                    if fail {
                        let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        continue;
                    }
                    sleep(Duration::from_secs(3)).await;
                }
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                    end - start + 1
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body[start..=end]).await;
            }
        });
        let source =
            range::RangeSource::open(reqwest::Client::new(), url, Default::default()).await?;
        let control = source.control.clone();
        let input = songbird::input::Input::Live(
            songbird::input::LiveInput::Raw(songbird::input::AudioStream {
                input: Box::new(source),
            }),
            None,
        );
        let (ticks, config) = songbird::Config::test_cfg(true);
        let mut driver = songbird::Driver::new(config);
        let (ended, mut outcome) = watch::channel(None);
        let handle = driver.play_only_input(input);
        handle.add_event(Event::Track(TrackEvent::End), End(ended))?;
        let ticker = tokio::spawn(async move {
            loop {
                ticks.tick(1);
                let _ = ticks.recv_async().await;
                sleep(Duration::from_millis(2)).await;
            }
        });
        timeout(Duration::from_secs(5), requested.notified()).await?;
        if fail {
            timeout(Duration::from_secs(5), outcome.changed()).await??;
            ensure!(
                *outcome.borrow() == Some(PlayMode::End),
                "upstream behavior changed"
            );
            ensure!(control.failed(), "lost source failure behind Songbird End");
            println!(
                "local 403: actual mixer End retains source.failed=true; live/controller regressions classify failure"
            );
        } else {
            ensure!(
                timeout(Duration::from_secs(2), handle.get_info())
                    .await
                    .is_err(),
                "expected blocked callback"
            );
            let state = timeout(Duration::from_secs(3), handle.get_info()).await??;
            ensure!(
                state.playing == PlayMode::Play && !control.failed(),
                "valid slow range must remain playing"
            );
            println!(
                "local 3-second range: info exceeds 2 seconds, then reports Play without source failure; controller grace is 15 seconds"
            );
        }
        control.stop();
        driver.stop();
        ticker.abort();
        server.abort();
        let _ = ticker.await;
        let _ = server.await;
    }
    Ok(())
}
