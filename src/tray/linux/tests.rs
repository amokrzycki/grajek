use super::*;
use crate::player::{self, Phase, State};
use anyhow::{Context as _, Result};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt as _, BufReader},
    process::Command,
    sync::mpsc,
    time::timeout,
};

struct Watcher(mpsc::UnboundedSender<String>);

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self.0.is_closed()
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus injects the message header by value"
    )]
    fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        assert_eq!(service, "/StatusNotifierItem");
        if let Some(sender) = header.sender() {
            let _ = self.0.send(sender.to_string());
        }
    }
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One private bus verifies icon, menu and registration lifecycle together"
)]
async fn icons_follow_watch_updates_without_reregistering_or_refreshing_unchanged_visuals()
-> Result<()> {
    let mut bus = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut address = String::new();
    timeout(
        Duration::from_secs(5),
        BufReader::new(bus.stdout.take().context("missing bus stdout")?).read_line(&mut address),
    )
    .await??;
    let address = address.trim();
    let (registered, mut registrations) = mpsc::unbounded_channel();
    let host = Builder::address(address)?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at("/StatusNotifierWatcher", Watcher(registered))?
        .build()
        .await?;
    let (states, receiver) = watch::channel(State {
        phase: Phase::Playing,
        available: true,
        controller_ready: true,
        voice_connected: true,
        track: Some(crate::media::TrackInfo {
            title: Some("Tray fixture".into()),
            duration: Some(Duration::from_secs(60)),
            seekable: true,
        }),
        ..State::default()
    });
    let (commands, mut intents) = mpsc::channel(16);
    let (window, mut windows) = mpsc::channel(16);
    let endpoint = Endpoint {
        player: crate::native::Endpoint {
            commands,
            states: receiver,
        },
        window,
        available: Arc::new(AtomicBool::new(false)),
    };
    let (stop, mut shutdown) = watch::channel(false);
    let builder = Builder::address(address)?;
    let server = tokio::spawn(async move { serve(endpoint, &mut shutdown, builder).await });
    let service = timeout(Duration::from_secs(5), registrations.recv())
        .await?
        .context("tray not registered")?;
    let client = Builder::address(address)?.build().await?;
    let item = zbus::Proxy::new(
        &client,
        service.as_str(),
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
    )
    .await?;
    // Query without the proxy property cache: NewIcon is the SNI invalidation signal.
    let properties = zbus::Proxy::new(
        &client,
        service.as_str(),
        "/StatusNotifierItem",
        "org.freedesktop.DBus.Properties",
    )
    .await?;
    let initial: OwnedValue = properties
        .call("Get", &("org.kde.StatusNotifierItem", "IconPixmap"))
        .await?;
    let initial: Vec<Pixmap> = initial.try_into()?;
    assert_eq!(
        initial.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(),
        [(16, 16), (22, 22), (32, 32), (48, 48)]
    );
    assert!(
        initial[0]
            .2
            .as_chunks::<4>()
            .0
            .contains(&[255, 221, 194, 139])
    );
    let mut icons = item.receive_signal("NewIcon").await?;
    let mut titles = item.receive_signal("NewTitle").await?;
    let menu =
        zbus::Proxy::new(&client, service.as_str(), "/Menu", "com.canonical.dbusmenu").await?;
    let mut layouts = menu.receive_signal("LayoutUpdated").await?;
    for (phase, changes_icon) in [
        (Phase::Paused, true),
        (Phase::Playing, true),
        (Phase::Error, true),
        (Phase::Ready, true),
        (Phase::Connecting, false),
        (Phase::Resolving, false),
        (Phase::Stopping, false),
        (Phase::Disconnecting, false),
        (Phase::Initializing, false),
        (Phase::Playing, true),
    ] {
        states.send_modify(|state| state.phase = phase);
        timeout(Duration::from_secs(2), titles.next())
            .await
            .with_context(|| format!("title timeout for {phase:?}"))?
            .context("missing title update")?;
        timeout(Duration::from_secs(2), layouts.next())
            .await
            .with_context(|| format!("menu timeout for {phase:?}"))?
            .context("missing menu update")?;
        if changes_icon {
            timeout(Duration::from_secs(2), icons.next())
                .await
                .with_context(|| format!("icon timeout for {phase:?}"))?
                .context("missing NewIcon")?;
        } else {
            assert!(
                timeout(Duration::from_millis(50), icons.next())
                    .await
                    .is_err()
            );
        }
        let current: OwnedValue = properties
            .call("Get", &("org.kde.StatusNotifierItem", "IconPixmap"))
            .await?;
        assert_eq!(
            Vec::<Pixmap>::try_from(current)?,
            pixmaps(Visual::from(&*states.borrow()))?
        );
    }
    states.send_modify(|state| {
        state.position = Duration::from_secs(42);
        state.import = player::ImportStatus::Failed("Independent import failed".into());
        state.error = Some("Stale error".into());
    });
    assert!(
        timeout(Duration::from_millis(100), icons.next())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(50), layouts.next())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(50), titles.next())
            .await
            .is_err()
    );
    menu.call::<_, _, ()>("Event", &(2_i32, "clicked", OwnedValue::from(0_i32), 0_u32))
        .await?;
    assert_eq!(
        timeout(Duration::from_secs(2), windows.recv())
            .await
            .context("Hide command timeout")?,
        Some(super::super::WindowAction::Hide)
    );
    menu.call::<_, _, ()>("Event", &(3_i32, "clicked", OwnedValue::from(0_i32), 0_u32))
        .await?;
    assert!(matches!(
        timeout(Duration::from_secs(2), intents.recv())
            .await
            .context("PlayPause command timeout")?,
        Some(player::Command::Media(crate::native::Command::PlayPause))
    ));
    assert!(
        registrations.try_recv().is_err(),
        "state updates must not register another item"
    );
    stop.send(true)?;
    timeout(Duration::from_secs(2), server)
        .await
        .context("server shutdown timeout")???;
    assert!(
        !fdo::DBusProxy::new(&client)
            .await?
            .name_has_owner(service.as_str().try_into()?)
            .await?
    );
    host.close().await?;
    Ok(())
}
