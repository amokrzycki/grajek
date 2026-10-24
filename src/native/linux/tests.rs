use super::*;
use crate::{
    media::TrackInfo,
    player::{self, Phase, State},
};
use anyhow::{Context, Result};
use futures_lite::StreamExt as _;
use tokio::{
    io::{AsyncBufReadExt as _, BufReader},
    process::Command as Process,
    sync::mpsc,
    time::timeout,
};

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One isolated bus exercises the complete registration and release lifecycle"
)]
async fn isolated_bus_exposes_interfaces_commands_properties_and_releases_name() -> Result<()> {
    let mut bus = Process::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut address = String::new();
    timeout(
        Duration::from_secs(5),
        BufReader::new(bus.stdout.take().context("D-Bus stdout missing")?).read_line(&mut address),
    )
    .await??;
    let address = address.trim();
    let (commands, mut received) = mpsc::channel(16);
    let playing = State {
        phase: Phase::Playing,
        available: true,
        controller_ready: true,
        voice_connected: true,
        track_id: 9,
        track: Some(TrackInfo {
            title: Some("Title".into()),
            duration: Some(Duration::from_secs(60)),
            seekable: true,
        }),
        position: Duration::from_secs(10),
        ..State::default()
    };
    let (states, updates) = watch::channel(State::default());
    let endpoint = Endpoint {
        commands,
        states: updates,
    };
    let server = connect(endpoint.clone(), Builder::address(address)?, NAME).await?;
    assert!(
        connect(endpoint.clone(), Builder::address(address)?, NAME)
            .await
            .is_err(),
        "name collision must not replace the first instance"
    );
    let client = Builder::address(address)?.build().await?;
    let root = zbus::Proxy::new(&client, NAME, PATH, "org.mpris.MediaPlayer2").await?;
    assert_eq!(root.get_property::<String>("Identity").await?, "Grajek");
    let player = zbus::Proxy::new(&client, NAME, PATH, "org.mpris.MediaPlayer2.Player").await?;
    // Discover before Discord startup: an ordinary caching client must stay controllable.
    assert!(player.get_property::<bool>("CanControl").await?);
    assert!(!player.get_property::<bool>("CanPlay").await?);
    let mut can_play_changes = player.receive_property_changed::<bool>("CanPlay").await;
    let initial = timeout(Duration::from_secs(2), can_play_changes.next())
        .await?
        .context("no initial cached value")?;
    assert!(!initial.get().await?);
    let before = endpoint.snapshot();
    states.send_replace(playing);
    publish(&server, &before, &endpoint.snapshot()).await?;
    let changed = timeout(Duration::from_secs(2), can_play_changes.next())
        .await?
        .context("no readiness change")?;
    assert!(changed.get().await?);
    assert!(
        player.get_property::<bool>("CanControl").await?,
        "cached support must remain true"
    );
    assert_eq!(
        player.get_property::<String>("PlaybackStatus").await?,
        "Playing"
    );
    assert!(!player.get_property::<bool>("CanGoPrevious").await?);
    assert!(player.get_property::<bool>("CanSeek").await?);
    let meta = player
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .await?;
    assert_eq!(
        <&str>::try_from(meta.get("xesam:title").context("no title")?)?,
        "Title"
    );
    assert_eq!(
        i64::try_from(meta.get("mpris:length").context("no length")?)?,
        60_000_000
    );
    for method in ["Play", "Pause", "PlayPause", "Next", "Stop"] {
        player.call::<_, _, ()>(method, &()).await?;
    }
    for check in ["play", "pause", "toggle", "next", "stop"] {
        let command = received.try_recv()?;
        assert!(matches!(
            (check, command),
            ("play", player::Command::Media(Command::Play))
                | ("pause", player::Command::Media(Command::Pause))
                | ("toggle", player::Command::Media(Command::PlayPause))
                | ("next", player::Command::Media(Command::Next))
                | ("stop", player::Command::Media(Command::Stop))
        ));
    }
    player.call::<_, _, ()>("Seek", &(-3_000_000_i64,)).await?;
    assert!(matches!(
        received.try_recv()?,
        player::Command::Media(Command::SeekRelative {
            track_id: 9,
            offset: -3_000_000
        })
    ));
    for (id, position) in [(8, 20_000_000_i64), (9, -1)] {
        player
            .call::<_, _, ()>("SetPosition", &(track_path(id), position))
            .await?;
        assert!(received.try_recv().is_err());
    }
    player
        .call::<_, _, ()>("SetPosition", &(track_path(9), 20_000_000_i64))
        .await?;
    assert!(
        matches!(received.try_recv()?, player::Command::Media(Command::Seek { track_id: 9, position }) if position == Duration::from_secs(20))
    );
    assert!(player.call::<_, _, ()>("Previous", &()).await.is_err());
    assert!(player.set_property("Volume", 0.5_f64).await.is_err());
    // OS requests have not changed the published controller state optimistically.
    assert_eq!(
        player.get_property::<String>("PlaybackStatus").await?,
        "Playing"
    );
    let mut seeked = player.receive_signal("Seeked").await?;
    let before = endpoint.snapshot();
    states.send_modify(|state| {
        state.phase = Phase::Paused;
        state.position = Duration::from_secs(19);
        state.seek_serial = 1;
    });
    publish(&server, &before, &endpoint.snapshot()).await?;
    let signal = timeout(Duration::from_secs(2), seeked.next())
        .await?
        .context("no Seeked")?;
    assert_eq!(signal.body().deserialize::<i64>()?, 19_000_000);
    // A fresh proxy avoids the client's asynchronously refreshed property cache.
    let fresh = zbus::Proxy::new(&client, NAME, PATH, "org.mpris.MediaPlayer2.Player").await?;
    assert_eq!(
        fresh.get_property::<String>("PlaybackStatus").await?,
        "Paused"
    );
    let before = endpoint.snapshot();
    states.send_modify(|state| {
        state.track = None;
        state.phase = Phase::Ready;
    });
    publish(&server, &before, &endpoint.snapshot()).await?;
    let fresh = zbus::Proxy::new(&client, NAME, PATH, "org.mpris.MediaPlayer2.Player").await?;
    assert!(
        fresh
            .get_property::<HashMap<String, OwnedValue>>("Metadata")
            .await?
            .is_empty()
    );
    drop(can_play_changes);
    drop(seeked);
    drop(player);
    drop(root);
    drop(fresh);
    server.close().await?;
    let replacement = connect(endpoint, Builder::address(address)?, NAME).await?;
    replacement.close().await?;
    client.close().await?;
    bus.kill().await?;
    Ok(())
}

#[test]
fn control_support_is_constant_across_startup_and_failure_and_track_ids_are_application_owned() {
    let (commands, _received) = mpsc::channel(1);
    let (states, updates) = watch::channel(State::default());
    let player = Player(Endpoint {
        commands,
        states: updates,
    });
    assert!(player.can_control());
    assert!(!player.can_play());
    states.send_modify(|state| {
        state.phase = Phase::Ready;
        state.controller_ready = true;
        state.available = true;
    });
    assert!(player.can_control());
    states.send_modify(|state| {
        state.phase = Phase::Error;
        state.available = false;
    });
    assert!(player.can_control());
    assert!(!player.can_play());
    assert_eq!(
        track_path(9).as_str(),
        "/io/github/amokrzycki/grajek/track/t9"
    );
}

#[tokio::test]
async fn unavailable_bus_fails_without_retrying_or_panicking() -> Result<()> {
    let (commands, _received) = mpsc::channel(1);
    let (_states, updates) = watch::channel(State::default());
    let endpoint = Endpoint {
        commands,
        states: updates,
    };
    let address = format!(
        "unix:path={}/grajek-no-bus-{}",
        std::env::temp_dir().display(),
        std::process::id()
    );
    assert!(
        timeout(
            Duration::from_secs(2),
            connect(endpoint, Builder::address(address.as_str())?, NAME)
        )
        .await?
        .is_err()
    );
    Ok(())
}
