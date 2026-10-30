//! Opt-in offline native window: no Discord, configuration, network, or persisted files.
use super::*;
use gpui_kit::{TitlebarOptions, application};

#[test]
#[ignore = "opens an offline native window for 12 seconds; needs compositor access"]
#[expect(
    clippy::print_stderr,
    reason = "opt-in native check reports renderer/capture errors"
)]
fn native_offline_animation_check() {
    let mut snapshot = with_lists(Phase::Playing, 100, 50);
    snapshot.track = playing(Phase::Playing, Some("Offline native check"), 0).track;
    snapshot.track_id = 1;
    for item in &mut snapshot.queue {
        item.title = Some("Long queue title · Żółć 🎵 ".repeat(15));
    }
    for item in &mut snapshot.history {
        item.title = Some("Long history title · Żółć 🎵 ".repeat(15));
    }
    let (commands, mut receiver) = mpsc::channel(16);
    let (volume, _volume_rx) = watch::channel(VolumeRequest::default());
    let (states, updates) = watch::channel(snapshot.clone());
    application().with_assets(crate::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(420.), px(480.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Grajek · offline animation check".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| PlayerApp::new(window, cx, commands, volume, updates))
        });
        let (window, view) = match result {
            Ok(window) => window,
            Err(error) => {
                eprintln!("NATIVE OPEN FAILED: {error}");
                cx.quit();
                return;
            }
        };
        eprintln!(
            "NATIVE WINDOW OPENED: 420x480, 100 queue / 50 history, offline Playing, 1Hz updates"
        );
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            for frame in 0..720 {
                executor.timer(Duration::from_nanos(16_666_667)).await;
                while let Ok(command) = receiver.try_recv() {
                    let command = match command {
                        Command::Transport { id, command } => {
                            snapshot.transport_ack = id;
                            *command
                        }
                        command => command,
                    };
                    match command {
                        Command::Pause => snapshot.phase = Phase::Paused,
                        Command::Resume => snapshot.phase = Phase::Playing,
                        Command::Play(_) => {
                            snapshot.phase = Phase::Playing;
                            snapshot.track_id += 1;
                        }
                        Command::Dequeue(id) => snapshot.queue.retain(|item| item.id != id),
                        Command::ForgetHistory(url) => {
                            snapshot.history.retain(|entry| entry.url != url);
                        }
                        _ => {}
                    }
                    states.send_replace(snapshot.clone());
                }
                if frame % 60 == 0 {
                    snapshot.position += Duration::from_secs(1);
                    states.send_replace(snapshot.clone());
                }
                if frame == 180 || frame == 360 {
                    let name = if frame == 180 {
                        "plans/native-queue.png"
                    } else {
                        "plans/native-history.png"
                    };
                    let capture = cx.update_window(window, |_, window, _| {
                        window
                            .render_to_image()
                            .and_then(|image| image.save(name).map_err(Into::into))
                    });
                    eprintln!("NATIVE CAPTURE {name}: {capture:?}");
                    if frame == 180 {
                        view.update(cx, |app, cx| app.select_section(Section::History, cx));
                    }
                }
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

/// Capture actual GPUI pixels on the host compositor using labelled offline snapshots.
#[test]
#[ignore = "opens an offline native state matrix; needs compositor and GPU access"]
#[expect(
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "opt-in native state matrix keeps compositor and capture lifecycle together and fails on capture errors"
)]
fn native_player_redesign_evidence() {
    use crate::player::{ImportStatus, RepeatMode};
    let directory = std::path::PathBuf::from(".impeccable/review/redesign");
    std::fs::create_dir_all(&directory).expect("capture directory");
    let (commands, _receiver) = mpsc::channel(16);
    let (volume, _volume_rx) = watch::channel(VolumeRequest::default());
    let (snapshots, updates) = watch::channel(state(Phase::Ready));
    application().with_assets(crate::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        let (window, view) = gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(520.), px(560.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("Grajek · offline UX evidence".into()), ..Default::default() }),
            ..Default::default()
        }, cx, |window, cx| cx.new(|cx| {
            let mut app = PlayerApp::new(window, cx, commands, volume, updates);
            app.setup_starting = false;
            app.settings_open = false;
            window.focus(&app.input.read(cx).focus_handle(cx), cx);
            app
        })).expect("native window");
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            for (width, height) in [(520., 560.), (420., 480.)] {
                cx.update_window(window, |_, window, _| window.resize(size(px(width), px(height)))).expect("resize");
                executor.timer(Duration::from_millis(350)).await;
                for name in ["idle", "playing", "paused", "connecting", "resolving", "importing", "error", "disconnected", "full", "history", "options", "more", "setup"] {
                    let mut snapshot = if name == "setup" {
                        State::default()
                    } else if name == "idle" {
                        state(Phase::Ready)
                    } else {
                        let phase = match name {
                            "paused" | "importing" => Phase::Paused,
                            "resolving" => Phase::Resolving,
                            "connecting" => Phase::Connecting,
                            "error" | "disconnected" => Phase::Error,
                            _ => Phase::Playing,
                        };
                        let mut snapshot = with_lists(phase, 12, 12);
                        snapshot.track = playing(phase, Some("Offline fixture · A quieter kind of music"), 84).track;
                        snapshot.track_id = 1;
                        snapshot.position = Duration::from_secs(84);
                        snapshot.voice_connected = true;
                        snapshot
                    };
                    if name != "setup" {
                        snapshot.setup.destination = Some(crate::setup::Destination {
                            bot: serenity::all::UserId::new(1),
                                guild: serenity::all::GuildId::new(2), channel: serenity::all::ChannelId::new(3),
                            guild_name: "Offline fixture · Friends' server".into(),
                            channel_name: "Music room".into(),
                        });
                    }
                    if name == "importing" {
                        snapshot.import = ImportStatus::Importing;
                        snapshot.error = Some("Offline fixture: the previous seek failed. Playback remains paused. Try a different position.".into());
                    }
                    if name == "error" {
                        snapshot.error = Some("Offline fixture: unable to play this video. Check the link, then try Play now or the next queued track.".into());
                        snapshot.queue[0].title = Some("Żółć Привет 🎵 a long queue title ".repeat(20));
                    }
                    if name == "disconnected" {
                        snapshot.available = false;
                        snapshot.voice_connected = false;
                        snapshot.track = None;
                    }
                    if name == "full" { snapshot.queue = with_lists(Phase::Playing, 100, 0).queue; }
                    if name == "options" { snapshot.preferences.repeat = RepeatMode::Queue; }
                    let expected_phase = snapshot.phase;
                    let expected_count = snapshot.queue.len();
                    cx.update_window(window, |_, window, cx| view.update(cx, |app, cx| {
                        app.url_actions_open = name == "more";
                        app.settings_open = name == "setup";
                        app.select_section(if name == "history" { Section::History } else { Section::Queue }, cx);
                        if name == "setup" { window.focus(&app.token.read(cx).focus_handle(cx), cx); }
                        window.refresh();
                        window.on_next_frame(|_, _| {});
                    })).expect("configure fixture surface");
                    snapshots.send_replace(snapshot);
                    executor.timer(Duration::from_millis(500)).await;
                    if name == "options" {
                        cx.update_window(window, |_, window, cx| window.click("queue-options", cx)).expect("open native options");
                        executor.timer(Duration::from_millis(500)).await;
                    }
                    // Settle X11's buffered presentation before reading native pixels.
                    for _ in 0..3 {
                        let (presented, frame) = tokio::sync::oneshot::channel();
                        cx.update_window(window, |_, window, _| {
                            window.refresh();
                            window.on_next_frame(move |_, _| { let _ = presented.send(()); });
                        }).expect("request capture frame");
                        let rendered = futures_lite::future::race(async { frame.await.is_ok() }, async {
                            executor.timer(Duration::from_secs(3)).await;
                            false
                        }).await;
                        assert!(rendered, "native fixture must render the requested state");
                        executor.timer(Duration::from_millis(100)).await;
                    }
                    cx.update_window(window, |_, window, cx| {
                        assert_eq!(view.read(cx).state.phase, expected_phase, "{name}: controller snapshot applied");
                        assert_eq!(view.read(cx).state.queue.len(), expected_count);
                        if name == "setup" { assert!(window.try_find("bot-token").is_some()); } else {
                            assert_eq!(window.find("pause-resume").label(), Some(if expected_phase == Phase::Paused { "Resume" } else { "Pause" }), "{name}: rendered transport matches snapshot");
                        }
                        if name == "history" { assert!(window.try_find("history-count").is_some()); }
                    }).expect("verify rendered fixture state");
                    let path = directory.join(format!("{name}-{width:.0}.png"));
                    #[cfg(target_os = "linux")]
                    {
                        // Linux has no render_to_image; KDE captures the displayed frame rather than a stale X11 backing buffer.
                        let found = std::process::Command::new("xdotool")
                            .args(["search", "--pid", &std::process::id().to_string(), "--name", "Grajek"])
                            .output().expect("find native X11 fixture");
                        let ids = String::from_utf8(found.stdout).expect("X11 window IDs");
                        let id = ids.lines().last().expect("native fixture window ID");
                        // The test driver selects only its own window, as in scripts/check-tray-linux.py.
                        let activation = r"import dbus, sys, tempfile, time
pid = int(sys.argv[1])
bus = dbus.SessionBus()
kwin = dbus.Interface(bus.get_object('org.kde.KWin', '/Scripting'), 'org.kde.kwin.Scripting')
plugin = 'grajek-ux-capture-' + str(pid)
with tempfile.NamedTemporaryFile(mode='w', suffix='.js') as script:
    script.write('const w = workspace.windowList().filter(w => w.pid === ' + str(pid) + '); if (w.length === 1) workspace.activeWindow = w[0];')
    script.flush()
    idx = bus.call_blocking('org.kde.KWin', '/Scripting', 'org.kde.kwin.Scripting', 'loadScript', 'ss', (script.name, plugin))
    try:
        dbus.Interface(bus.get_object('org.kde.KWin', '/Scripting/Script' + str(idx)), 'org.kde.kwin.Script').run()
        time.sleep(0.2)
    finally:
        kwin.unloadScript(plugin)
";
                        assert!(std::process::Command::new("python3").args(["-c", activation, &std::process::id().to_string()]).status().expect("select fixture window").success());
                        executor.timer(Duration::from_millis(200)).await;
                        let active = std::process::Command::new("xdotool").arg("getactivewindow").output().expect("active fixture");
                        assert_eq!(String::from_utf8(active.stdout).expect("active window ID").trim(), id, "capture only the fixture");
                        let mut capture = std::process::Command::new("spectacle")
                            .env("WAYLAND_DISPLAY", std::env::var("GRAJEK_CAPTURE_WAYLAND_DISPLAY").expect("set GRAJEK_CAPTURE_WAYLAND_DISPLAY to the compositor socket"))
                            .env("QT_QPA_PLATFORM", "wayland")
                            .args(["--background", "--nonotify", "--activewindow", "--no-decoration", "--no-shadow", "--output"])
                            .arg(&path).spawn().expect("capture compositor pixels");
                        let mut captured = None;
                        for _ in 0..100 {
                            captured = capture.try_wait().expect("capture status");
                            if captured.is_some() { break; }
                            executor.timer(Duration::from_millis(50)).await;
                        }
                        if captured.is_none() { capture.kill().expect("stop stalled capture"); }
                        assert!(captured.is_some_and(|status| status.success()), "compositor capture failed");
                    }
                    #[cfg(not(target_os = "linux"))]
                    cx.update_window(window, |_, window, _| window.render_to_image().expect("native pixels").save(&path).expect("save native image")).expect("capture window");
                    cx.update_window(window, |_, window, cx| window.press("escape", cx)).expect("dismiss menus");
                }
            }
            cx.update(|cx| cx.quit());
        }).detach();
    });
}
