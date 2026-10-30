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
    clippy::panic,
    reason = "opt-in native state matrix keeps compositor and capture lifecycle together and fails on capture errors"
)]
fn native_player_redesign_evidence() {
    use crate::player::{ImportStatus, RepeatMode};
    use gpui_kit::component::ActiveTheme;
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let directory = std::env::var_os("GRAJEK_CAPTURE_DIR").map_or_else(
        || std::path::PathBuf::from(".impeccable/review/refinement/after"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&directory).expect("capture directory");
    let (commands, mut receiver) = mpsc::channel(16);
    let (volume, _volume_rx) = watch::channel(VolumeRequest::default());
    let (snapshots, updates) = watch::channel(state(Phase::Ready));
    application().with_assets(crate::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        std::fs::write(directory.join("renderer.txt"), format!(
            "OS: {}\nFont: {}\nMono: {}\nRem: {:?}\nOffline snapshots; no Discord audio.\n",
            std::env::consts::OS, cx.theme().font_family, cx.theme().mono_font_family, cx.theme().font_size,
        )).expect("renderer metadata");
        let dimensions = if std::env::var("GRAJEK_CAPTURE_WIDTH").is_ok_and(|width| width == "420") { size(px(420.), px(480.)) } else { size(px(520.), px(560.)) };
        let mut initial_bounds = Bounds::centered(None, dimensions, cx);
        if let Ok(corner) = std::env::var("GRAJEK_CAPTURE_CORNER") && let Some(display) = cx.primary_display() {
            let screen = display.bounds();
            initial_bounds.origin = if corner == "bottom-right" { gpui_kit::point(screen.right() - dimensions.width, screen.bottom() - dimensions.height) } else { screen.origin };
        }
        let (window, view) = gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(initial_bounds)),
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
            let mut cases = vec!["idle", "playing", "playing-empty", "focused", "unfocused", "paused", "connecting", "resolving", "importing", "error", "disconnected", "full", "history", "options", "setup", "settings", "destination", "credentials", "help", "options-hover", "server-selector", "channel-selector", "selector-keyboard", "permission", "setup-help", "auth-error", "idle-queued", "long-title", "link-video", "link-playlist", "added", "reordered", "removed", "removed-keyboard", "settings-long", "onboarding-player", "destination-player", "settings-scroll", "confirmation", "help-bottom"];
            if std::env::var_os("GRAJEK_CAPTURE_INTERACTIONS").is_some() {
                cases.extend(["hover-input", "hover-button", "pressed-button", "keyboard-button", "keyboard-row"]);
            }
            if let Ok(filter) = std::env::var("GRAJEK_CAPTURE_CASES") {
                cases.retain(|name| filter.split(',').any(|selected| selected == *name));
            }
            for (width, height) in [(520., 560.), (420., 480.)] {
                if std::env::var("GRAJEK_CAPTURE_WIDTH").is_ok_and(|filter| filter != format!("{width:.0}")) { continue; }
                cx.update_window(window, |_, window, _| window.resize(size(px(width), px(height)))).expect("resize");
                executor.timer(Duration::from_millis(350)).await;
                #[cfg(target_os = "linux")]
                if std::env::var_os("GRAJEK_CAPTURE_CORNER").is_some() {
                    // KWin places new XWayland windows itself; request an explicit move after mapping.
                    let mut position = std::process::Command::new("xdotool").args([
                        "search", "--pid", &std::process::id().to_string(), "--name", "offline UX evidence",
                        "windowmove", "--sync", "%1", &format!("{:.0}", initial_bounds.origin.x.as_f32()), &format!("{:.0}", initial_bounds.origin.y.as_f32())
                    ]).spawn().expect("move only the offline fixture to the requested monitor edge");
                    let mut status = None;
                    for _ in 0..100 {
                        status = position.try_wait().expect("fixture position status");
                        if status.is_some() { break; }
                        executor.timer(Duration::from_millis(20)).await;
                    }
                    if status.is_none() { position.kill().expect("stop stalled fixture move"); }
                    assert!(status.is_some_and(|status| status.success()), "compositor fixture move failed");
                    executor.timer(Duration::from_millis(250)).await;
                }
                for name in cases.iter().copied() {
                    let mut snapshot = if matches!(name, "setup" | "setup-help" | "auth-error" | "onboarding-player") {
                        let mut snapshot = State::default();
                        if matches!(name, "auth-error" | "onboarding-player") { snapshot.controller_ready = true; snapshot.phase = Phase::Ready; }
                        if name == "auth-error" { snapshot.setup.status = crate::setup::Status::Failed("Discord rejected this bot token. Copy the token from the Bot page in the Developer Portal, then connect again.".into()); }
                        snapshot
                    } else if matches!(name, "settings" | "destination" | "credentials" | "server-selector" | "channel-selector" | "selector-keyboard" | "permission" | "settings-long" | "destination-player" | "settings-scroll" | "confirmation") {
                        let mut snapshot = super::experience::configured();
                        if matches!(name, "destination" | "destination-player") { snapshot.setup.destination = None; snapshot.available = false; snapshot.setup.status = crate::setup::Status::NeedsDestination; }
                        snapshot
                    } else if matches!(name, "idle" | "idle-queued") {
                        let mut snapshot = super::experience::configured();
                        if name == "idle-queued" { snapshot.queue = with_lists(Phase::Ready, 3, 0).queue; }
                        snapshot
                    } else {
                        let phase = match name {
                            "paused" | "importing" => Phase::Paused,
                            "resolving" => Phase::Resolving,
                            "connecting" => Phase::Connecting,
                            "error" | "disconnected" => Phase::Error,
                            _ => Phase::Playing,
                        };
                        let mut snapshot = with_lists(phase, 12, 12);
                        snapshot.track = playing(phase, Some("Nils Frahm — Says (Live at the Funkhaus Berlin)"), 84).track;
                        for (index, item) in snapshot.queue.iter_mut().enumerate() {
                            let titles = [
                                "Ólafur Arnalds — Near Light (Official Music Video)",
                                "Hania Rani — Glass | Live from Studio S2",
                                "坂本龍一 — Merry Christmas Mr. Lawrence",
                                "Portico Quartet — Prickly Pear",
                                "GoGo Penguin — Hopopono (Live at the Roundhouse)",
                                "Żółć · Привет · A very long title that should truncate gracefully",
                            ];
                            item.title = Some(titles[index % titles.len()].into());
                        }
                        snapshot.track_id = 1;
                        snapshot.position = Duration::from_secs(84);
                        snapshot.voice_connected = true;
                        snapshot
                    };
                    if !matches!(name, "setup" | "setup-help" | "auth-error" | "onboarding-player" | "settings" | "destination" | "credentials" | "server-selector" | "channel-selector" | "selector-keyboard" | "permission" | "settings-long" | "destination-player" | "settings-scroll" | "confirmation") {
                        snapshot.setup.destination = Some(crate::setup::Destination {
                            bot: serenity::all::UserId::new(1),
                                guild: serenity::all::GuildId::new(2), channel: serenity::all::ChannelId::new(3),
                            guild_name: "Friends’ listening room".into(),
                            channel_name: "Music room".into(),
                        });
                    }
                    if name == "permission" { snapshot.setup.channels[2].name = "Private listening room · 夜の音楽 · A very long unavailable channel name".into(); }
                    if name == "confirmation" { snapshot.confirm_setup = true; snapshot.phase = Phase::Paused; snapshot.track = playing(Phase::Paused, Some("Nils Frahm — Says"), 84).track; snapshot.voice_connected = true; }
                    if name == "help-bottom" { snapshot.queue[if width < 500. { 3 } else { 4 }].title = Some("A longer title near the window boundary · Żółć · 音楽を楽しむ · Привет".into()); }
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
                    if name == "playing-empty" { snapshot.queue.clear(); }
                    if name == "long-title" { snapshot.track = playing(Phase::Playing, Some("Nils Frahm & Ólafur Arnalds — Collaborative Improvisation Sessions, Full Concert Recording (Live at the Funkhaus Berlin, 2019 — Remastered)"), 84).track; }
                    if name == "full" { snapshot.queue = with_lists(Phase::Playing, 100, 0).queue; }
                    if name == "options" { snapshot.preferences.repeat = RepeatMode::Queue; }
                    let expected_phase = snapshot.phase;
                    let mut expected_count = snapshot.queue.len();
                    let after_commands = snapshot.clone();
                    cx.update_window(window, |_, window, cx| view.update(cx, |app, cx| {
                        app.settings_open = !matches!(name, "onboarding-player" | "destination-player") && matches!(name, "setup" | "setup-help" | "auth-error" | "onboarding-player" | "settings" | "destination" | "credentials" | "server-selector" | "channel-selector" | "selector-keyboard" | "permission" | "settings-long" | "destination-player" | "settings-scroll" | "confirmation");
                        app.credentials_open = matches!(name, "credentials" | "settings-scroll");
                        app.preferences_open = matches!(name, "settings-long" | "settings-scroll");
                        app.input.update(cx, |input, cx| input.set_value("", window, cx));
                        app.added = None;
                        app.enqueue_pending = None;
                        app.local_hint = None;
                        app.settings_scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                        app.setup_help = matches!(name, "setup-help" | "settings-long" | "settings-scroll");
                        app.help.dismiss(cx);
                        app.select_section(if name == "history" { Section::History } else { Section::Queue }, cx);
                        if matches!(name, "setup" | "setup-help" | "auth-error" | "credentials") {
                            window.focus(&app.token.read(cx).focus_handle(cx), cx);
                        } else if name == "focused" {
                            window.focus(&app.input.read(cx).focus_handle(cx), cx);
                        } else {
                            window.blur(cx);
                        }
                        window.refresh();
                        window.on_next_frame(|_, _| {});
                    })).expect("configure fixture surface");
                    snapshots.send_replace(snapshot);
                    executor.timer(Duration::from_millis(500)).await;
                    if matches!(name, "link-video" | "link-playlist" | "added") {
                        let link = if name == "link-playlist" { "https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG" } else { "https://youtu.be/aqz-KE-bpKQ" };
                        cx.update_window(window, |_, window, cx| {
                            view.update(cx, |app, cx| window.focus(&app.input.read(cx).focus_handle(cx), cx));
                            window.input(link, cx);
                            if name == "added" { window.press("ctrl-enter", cx); }
                        }).expect("type a link");
                        if name == "added" {
                            executor.timer(Duration::from_millis(150)).await;
                            assert!(matches!(receiver.try_recv(), Ok(Command::Enqueue(url)) if url == link), "added: Enqueue intent sent");
                            let mut acknowledged = after_commands.clone();
                            let id = acknowledged.queue.iter().map(|item| item.id).max().unwrap_or(0) + 1;
                            acknowledged.queue.push(crate::queue::QueueItem { id, url: link.into(), title: Some("Aqua Mundi — Live from the Funkhaus (acknowledged by the controller)".into()), title_unavailable: false });
                            expected_count = acknowledged.queue.len();
                            snapshots.send_replace(acknowledged);
                            executor.timer(Duration::from_millis(300)).await;
                        }
                    }
                    if matches!(name, "reordered" | "removed" | "removed-keyboard") {
                        let ids: Vec<u64> = after_commands.queue.iter().map(|item| item.id).collect();
                        let mut acknowledged = after_commands.clone();
                        if name == "reordered" {
                            cx.update_window(window, |_, window, cx| window.click(("queue-down", ids[0]), cx)).expect("move first row down");
                            executor.timer(Duration::from_millis(150)).await;
                            assert!(matches!(receiver.try_recv(), Ok(Command::Shift { id, up: false }) if id == ids[0]), "reordered: Shift intent sent");
                            acknowledged.queue.swap(0, 1);
                        } else {
                            if name == "removed-keyboard" {
                                // Keyboard removal: focus the second row's Remove by Tab, then press Delete.
                                cx.update_window(window, |_, window, cx| {
                                    view.update(cx, |app, cx| window.focus(&app.input.read(cx).focus_handle(cx), cx));
                                    for _ in 0..40 {
                                        if window.find(("queue-remove", ids[1])).focused() == Some(true) { break; }
                                        window.press("tab", cx);
                                    }
                                    assert_eq!(window.find(("queue-remove", ids[1])).focused(), Some(true), "keyboard focus reached Remove");
                                    window.press("delete", cx);
                                }).expect("remove second row from the keyboard");
                            } else {
                                cx.update_window(window, |_, window, cx| window.click(("queue-remove", ids[1]), cx)).expect("remove second row");
                            }
                            executor.timer(Duration::from_millis(150)).await;
                            assert!(matches!(receiver.try_recv(), Ok(Command::Dequeue(id)) if id == ids[1]), "removed: Dequeue intent sent");
                            acknowledged.queue.remove(1);
                            expected_count = acknowledged.queue.len();
                        }
                        snapshots.send_replace(acknowledged);
                        executor.timer(Duration::from_millis(300)).await;
                    }
                    if matches!(name, "options" | "options-hover") {
                        cx.update_window(window, |_, window, cx| window.click("queue-options", cx)).expect("open native options");
                        executor.timer(Duration::from_millis(500)).await;
                    }
                    cx.update_window(window, |_, window, cx| {
                        window.simulate_mouse_move(gpui_kit::point(px(2.), px(2.)), cx);
                        if name == "hover-input" { window.hover("url", cx); }
                        if matches!(name, "hover-button" | "pressed-button") {
                            window.hover("pause-resume", cx);
                        }
                        if name == "pressed-button" {
                            window.dispatch_event(MouseDownEvent {
                                button: MouseButton::Left,
                                position: window.find("pause-resume").bounds().center(),
                                ..Default::default()
                            }.to_platform_input(), cx);
                        }
                        if matches!(name, "keyboard-button" | "keyboard-row") {
                            view.update(cx, |app, cx| window.focus(&app.input.read(cx).focus_handle(cx), cx));
                            let target: ElementId = if name == "keyboard-button" {
                                "pause-resume".into()
                            } else { ("queue-down", 10_u64).into() };
                            for _ in 0..32 {
                                if window.find(target.clone()).focused() == Some(true) { break; }
                                window.press(if name == "keyboard-button" { "shift-tab" } else { "tab" }, cx);
                            }
                            assert_eq!(window.find(target).focused(), Some(true), "{name}: native keyboard focus reached target");
                        }
                    }).expect("configure native pointer and keyboard state");
                    if matches!(name, "server-selector" | "channel-selector" | "selector-keyboard" | "permission") {
                        cx.update_window(window, |_, window, cx| {
                            if name == "server-selector" { window.click("server-selector", cx); }
                            else { window.click("channel-selector", cx); }
                            if name == "selector-keyboard" { window.press("down", cx); }
                        }).expect("open native destination selector");
                        executor.timer(Duration::from_millis(250)).await;
                    }
                    if name == "settings-scroll" {
                        cx.update_window(window, |_, window, cx| {
                            window.blur(cx);
                            window.scroll("discord-settings", gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1000.))), cx);
                        }).expect("scroll longer settings");
                    }
                    if name == "help-bottom" {
                        cx.update_window(window, |_, window, cx| window.hover(("queue-title", if width < 500. { 13_u64 } else { 14_u64 }), cx)).expect("hover boundary help");
                        executor.timer(Duration::from_millis(800)).await;
                    }
                    if matches!(name, "help" | "options-hover") {
                        cx.update_window(window, |_, window, cx| window.hover(if name == "help" { "title" } else { "continue-on-error" }, cx)).expect("hover native help");
                        executor.timer(Duration::from_millis(800)).await;
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
                        if view.read(cx).settings_open { assert!(window.try_find("bot-token").is_some() || window.try_find("server-selector").is_some() || window.try_find("confirm-setup").is_some()); } else if !view.read(cx).idle() {
                            assert_eq!(window.find("pause-resume").label(), Some(if expected_phase == Phase::Paused { "Resume" } else { "Pause" }), "{name}: rendered transport matches snapshot");
                        }
                        if name == "history" { assert!(window.try_find("history-count").is_some()); }
                    }).expect("verify rendered fixture state");
                    let geometry = cx.update_window(window, |_, window, _| {
                        let mut geometry = serde_json::Map::new();
                        let b = window.bounds();
                        geometry.insert("window-on-display".into(), serde_json::json!({ "x": b.origin.x.as_f32(), "y": b.origin.y.as_f32(), "width": b.size.width.as_f32(), "height": b.size.height.as_f32() }));
                        for id in ["player-header", "active-player", "idle-player", "link-entry", "url", "play", "pause-resume", "next", "queue-region", "queue-options-menu", "help-popup", "discord-settings", "server-selector", "channel-selector", "active-destination"] {
                            if let Some(element) = window.try_find(id) {
                                let b = element.bounds();
                                geometry.insert(id.into(), serde_json::json!({ "x": b.origin.x.as_f32(), "y": b.origin.y.as_f32(), "width": b.size.width.as_f32(), "height": b.size.height.as_f32() }));
                            }
                        }
                        geometry
                    }).expect("native geometry");
                    std::fs::write(directory.join(format!("{name}-{width:.0}.geometry.json")), serde_json::to_vec_pretty(&geometry).expect("geometry json")).expect("write geometry");
                    let path = directory.join(format!("{name}-{width:.0}.png"));
                    #[cfg(target_os = "linux")]
                    {
                        // Target the fixture UUID: active-window capture races desktop focus changes.
                        let mut capture = std::process::Command::new("python3")
                            .arg("scripts/capture-native-window.py")
                            .arg(std::process::id().to_string()).arg(&path)
                            .arg(format!("{width:.0}")).arg(format!("{height:.0}"))
                            .spawn().expect("capture fixture compositor pixels");
                        let mut captured = None;
                        for _ in 0..200 {
                            captured = capture.try_wait().expect("capture status");
                            if captured.is_some() { break; }
                            executor.timer(Duration::from_millis(50)).await;
                        }
                        if captured.is_none() { capture.kill().expect("stop stalled capture"); }
                        assert!(captured.is_some_and(|status| status.success()), "fixture compositor capture failed");
                    }
                    #[cfg(not(target_os = "linux"))]
                    cx.update_window(window, |_, window, _| window.render_to_image().expect("native pixels").save(&path).expect("save native image")).expect("capture window");
                    cx.update_window(window, |_, window, cx| {
                        if name == "pressed-button" {
                            window.dispatch_event(MouseUpEvent {
                                button: MouseButton::Left,
                                position: gpui_kit::point(px(2.), px(2.)),
                                ..Default::default()
                            }.to_platform_input(), cx);
                        }
                        window.press("escape", cx);
                    }).expect("dismiss menus and release pointer");
                }
            }
            while let Ok(command) = receiver.try_recv() {
                assert!(matches!(command, Command::ConfirmSetup(false)), "capture-state Escape may only cancel a destination confirmation");
            }
            // Exercise transport against the actual native event loop, without optimistic state.
            let mut response = playing(Phase::Playing, Some("Native interaction check"), 84);
            response.queue = with_lists(Phase::Playing, 2, 0).queue;
            cx.update_window(window, |_, _, cx| view.update(cx, |app, cx| {
                app.settings_open = false;
                app.select_section(Section::Queue, cx);
            })).expect("return to player");
            snapshots.send_replace(response.clone());
            executor.timer(Duration::from_millis(150)).await;
            cx.update_window(window, |_, window, cx| window.click("pause-resume", cx)).expect("native Pause click");
            executor.timer(Duration::from_millis(150)).await;
            let Command::Transport { id, command } = receiver.try_recv().expect("native transport intent") else { panic!("expected transport"); };
            assert!(matches!(*command, Command::Pause));
            cx.update_window(window, |_, window, _| {
                assert_eq!(window.find("pause-resume").label(), Some("Pause"));
                assert!(window.find("pause-resume").focused().is_none());
            }).expect("pending control retains confirmed state");
            response.phase = Phase::Paused;
            response.transport_ack = id;
            snapshots.send_replace(response);
            executor.timer(Duration::from_millis(150)).await;
            cx.update_window(window, |_, window, cx| {
                assert_eq!(window.find("pause-resume").label(), Some("Resume"));
                assert!(window.find("pause-resume").focused().is_some());
                view.update(cx, |app, cx| window.focus(&app.input.read(cx).focus_handle(cx), cx));
                window.input("https://youtu.be/aqz-KE-bpKQ", cx);
                window.press("ctrl-enter", cx);
            }).expect("native input and shortcut");
            assert!(matches!(receiver.try_recv(), Ok(Command::Enqueue(url)) if url == "https://youtu.be/aqz-KE-bpKQ"));
            std::fs::write(directory.join("interactions.txt"), "Native Pause: one intent; confirmed Playing retained while pending.\nAcknowledged Paused snapshot: Resume rendered and enabled.\nNative typing + Ctrl+Enter: Enqueue submitted with unchanged URL.\n").expect("interaction evidence");
            // Follow the real selector event/focus path, then acknowledge only controller snapshots.
            let mut destination = super::experience::configured();
            snapshots.send_replace(destination.clone());
            executor.timer(Duration::from_millis(150)).await;
            cx.update_window(window, |_, window, cx| {
                window.click("settings", cx);
                window.press("enter", cx);
                window.press("down", cx);
                window.press("enter", cx);
            }).expect("native server keyboard selection");
            executor.timer(Duration::from_millis(150)).await;
            assert!(matches!(receiver.try_recv(), Ok(Command::Setup(crate::setup::Request::Channels(id))) if id.get() == 4));
            destination.setup.browsing = Some(serenity::all::GuildId::new(4));
            snapshots.send_replace(destination.clone());
            executor.timer(Duration::from_millis(150)).await;
            cx.update_window(window, |_, window, cx| {
                window.click("channel-selector", cx);
                window.press("down", cx);
                window.press("down", cx);
                window.press("enter", cx);
            }).expect("native channel keyboard selection");
            executor.timer(Duration::from_millis(150)).await;
            assert!(matches!(receiver.try_recv(), Ok(Command::Setup(crate::setup::Request::Select(selected))) if selected.guild.get() == 4 && selected.channel.get() == 6));
            assert!(receiver.try_recv().is_err(), "selection alone must never send playback");
            destination.confirm_setup = true;
            snapshots.send_replace(destination);
            executor.timer(Duration::from_millis(150)).await;
            cx.update_window(window, |_, window, cx| {
                assert_eq!(window.find("confirm-setup").focused(), Some(true));
                window.press("escape", cx);
            }).expect("native destination cancellation");
            executor.timer(Duration::from_millis(100)).await;
            assert!(matches!(receiver.try_recv(), Ok(Command::ConfirmSetup(false))));
            std::fs::write(directory.join("selector-interactions.txt"), "Native keyboard server selection: Channels(guild 4), no destination/playback mutation.\nNative keyboard voice-channel selection: Select(guild 4, channel 6), no Play/Next/voice join.\nController confirmation focuses Stop and switch; Escape submits ConfirmSetup(false).\n").expect("selector interaction evidence");
            cx.update(|cx| cx.quit());
        }).detach();
    });
}
