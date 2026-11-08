//! Opt-in real download acceptance. Run only with a fresh, isolated LOCALAPPDATA.
use super::*;
use crate::media::components::{self, download::Progress, status::ComponentState};
use gpui_kit::{TitlebarOptions, application};

#[test]
#[ignore = "downloads pinned components from GitHub and opens a native Windows window; requires isolated LOCALAPPDATA"]
#[expect(
    clippy::expect_used,
    clippy::too_many_lines,
    reason = "Opt-in native acceptance fails with context and owns one application lifecycle"
)]
fn native_components_download_cancel_retry() {
    let fixture = std::path::PathBuf::from(
        std::env::var_os("GRAJEK_RECOVERY_FIXTURE_DIR").expect("set isolated fixture directory"),
    );
    assert_eq!(
        std::env::var_os("LOCALAPPDATA"),
        Some(fixture.clone().into_os_string())
    );
    assert!(fixture.is_absolute());
    assert!(std::env::var_os("GRAJEK_TOOLS").is_none());
    let tools = fixture.join("Grajek/tools");
    assert!(!tools.exists(), "use a fresh fixture directory");
    std::fs::create_dir_all(&fixture).expect("fixture directory");
    components::initialize_with_fallback(Some(tools.clone()), None);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("download runtime");
    let handle = runtime.handle().clone();
    let (commands, mut receiver) = mpsc::channel(16);
    let (volume, _volume_rx) = watch::channel(VolumeRequest::default());
    let (_states, updates) = watch::channel(state(Phase::Ready));
    application().with_assets(crate::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(420.), px(480.)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Grajek · isolated recovery acceptance".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    PlayerApp::new(window, cx, commands, volume, updates).with_components(handle)
                })
            },
        )
        .expect("native window");
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let started = std::time::Instant::now();
            loop {
                let checking = cx
                    .update_window(window, |_, _, cx| {
                        view.read(cx)
                            .components
                            .reports
                            .iter()
                            .any(|r| r.state == ComponentState::Checking)
                    })
                    .expect("check status");
                if !checking && started.elapsed() > Duration::from_millis(100) {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "initial check timed out"
                );
                executor.timer(Duration::from_millis(20)).await;
            }
            cx.update_window(window, |_, window, cx| {
                view.update(cx, PlayerApp::open_components);
                window.render_frame(cx);
            })
            .expect("open recovery");
            let started = std::time::Instant::now();
            loop {
                let checking = cx
                    .update_window(window, |_, _, cx| {
                        view.read(cx)
                            .components
                            .reports
                            .iter()
                            .any(|r| r.state == ComponentState::Checking)
                    })
                    .expect("recovery check status");
                if !checking {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "recovery check timed out"
                );
                executor.timer(Duration::from_millis(20)).await;
            }
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window
                    .render_to_image()
                    .expect("consent pixels")
                    .save(fixture.join("recovery-before-420.png"))
                    .expect("save consent screenshot");
                window.click("components-download", cx);
            })
            .expect("consent to download");
            let started = std::time::Instant::now();
            loop {
                let progress = cx
                    .update_window(window, |_, _, cx| view.read(cx).components.progress.clone())
                    .expect("download status");
                if matches!(progress, Progress::Downloading { bytes, .. } if bytes > 0) {
                    break;
                }
                assert!(
                    !matches!(progress, Progress::Failed(_)),
                    "download failed: {progress:?}"
                );
                assert!(
                    started.elapsed() < Duration::from_secs(45),
                    "download did not start: {progress:?}"
                );
                executor.timer(Duration::from_millis(20)).await;
            }
            cx.update_window(window, |_, window, cx| {
                window.click("components-cancel", cx);
            })
            .expect("cancel");
            let started = std::time::Instant::now();
            loop {
                let progress = cx
                    .update_window(window, |_, _, cx| view.read(cx).components.progress.clone())
                    .expect("cancel status");
                if progress == Progress::Cancelled {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "cancel did not settle: {progress:?}"
                );
                executor.timer(Duration::from_millis(20)).await;
            }
            assert!(!tools.join("components.json").exists());
            assert!(!tools.with_extension("partial").exists());
            cx.update_window(window, |_, window, cx| {
                window.click("components-download", cx);
            })
            .expect("retry");
            executor.timer(Duration::from_millis(250)).await;
            cx.update_window(window, |_, window, cx| window.click("components-later", cx))
                .expect("background download");
            assert!(
                !cx.update_window(window, |_, _, cx| view.read(cx).components.open)
                    .expect("closed panel")
            );
            cx.update_window(window, |_, _, cx| {
                view.update(cx, PlayerApp::open_components);
            })
            .expect("reopen progress");
            let started = std::time::Instant::now();
            loop {
                let (progress, ready) = cx
                    .update_window(window, |_, _, cx| {
                        let panel = &view.read(cx).components;
                        (
                            panel.progress.clone(),
                            panel
                                .reports
                                .iter()
                                .all(|r| r.state == ComponentState::Ready),
                        )
                    })
                    .expect("completion status");
                if progress == Progress::Complete && ready {
                    break;
                }
                assert!(
                    !matches!(progress, Progress::Failed(_)),
                    "retry failed: {progress:?}"
                );
                assert!(
                    started.elapsed() < Duration::from_secs(180),
                    "download did not complete: {progress:?}"
                );
                executor.timer(Duration::from_millis(100)).await;
            }
            assert!(tools.join("components.json").is_file());
            executor.timer(Duration::from_millis(200)).await;
            cx.update_window(window, |_, window, cx| {
                window
                    .render_to_image()
                    .expect("native recovery pixels")
                    .save(fixture.join("recovery-success-420.png"))
                    .expect("save recovery screenshot");
                window.click("components-later", cx);
            })
            .expect("return to player");
            assert!(
                receiver.try_recv().is_err(),
                "repair must not start playback"
            );
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    drop(runtime);
}
