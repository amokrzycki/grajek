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
