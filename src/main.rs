#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod discord;
mod live;
mod media;
mod native;
mod player;
mod playlist;
mod queue;
mod setup;
mod store;
mod style;
mod tray;
mod window;

use anyhow::{Context as _, Result};
use gpui_kit::{
    AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, assets, init, open_window,
    px, size,
};
use std::{borrow::Cow, time::Duration};
use tokio::sync::{mpsc, watch};

// Extend the default Lucide assets with the speaker and transport icons we use.
gpui_kit::assets::icon_assets!(ControlIcons, [Volume2, VolumeX, Pause, SkipForward, Square]);

struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match ControlIcons.load(path)? {
            Some(icon) => Ok(Some(icon)),
            None => assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut paths = assets::Assets.list(path)?;
        paths.extend(ControlIcons.list(path)?);
        Ok(paths)
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "grajek=info,warn".into()),
        )
        .init();
    let config = config::Config::load();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Could not start Tokio")?;
    let (commands, receiver) = mpsc::channel(16);
    let (volume, volume_rx) = watch::channel(player::VolumeRequest::default());
    let (states, updates) = watch::channel(player::State::default());
    let (shutdown, shutdown_rx) = watch::channel(false);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let endpoint = native::Endpoint {
        commands: commands.clone(),
        states: updates.clone(),
    };
    let (window_commands, window_requests) = mpsc::channel(16);
    let tray_endpoint = tray::Endpoint {
        player: endpoint.clone(),
        window: window_commands,
        available: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    #[cfg(target_os = "linux")]
    let tray_session = runtime.spawn(tray::linux::run(tray_endpoint.clone(), shutdown_rx.clone()));
    #[cfg(target_os = "linux")]
    let media_session = runtime.spawn(native::linux::run(endpoint.clone(), shutdown_rx.clone()));
    let controller = runtime.spawn(player::run(
        config,
        receiver,
        volume_rx,
        states,
        shutdown_rx,
    ));
    run_app(
        commands,
        volume,
        updates,
        endpoint,
        tray_endpoint,
        window_requests,
    );
    shutdown.send_replace(true);
    runtime.block_on(async {
        #[cfg(target_os = "linux")]
        {
            let mut tray_session = tray_session;
            if tokio::time::timeout(Duration::from_secs(2), &mut tray_session)
                .await
                .is_err()
            {
                tray_session.abort();
            }
            let mut media_session = media_session;
            if tokio::time::timeout(Duration::from_secs(2), &mut media_session)
                .await
                .is_err()
            {
                media_session.abort();
            }
        }
        let mut controller = controller;
        match tokio::time::timeout(Duration::from_secs(8), &mut controller).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "Playback controller failed"),
            Err(_) => {
                tracing::warn!("Controller shutdown timed out");
                controller.abort();
            }
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(2));
    Ok(())
}

fn window_options(cx: &gpui_kit::App) -> WindowOptions {
    WindowOptions {
        app_id: Some("org.grajek.desktop".into()),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(520.), px(560.)),
            cx,
        ))),
        window_min_size: Some(size(px(420.), px(480.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Grajek".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn run_app(
    commands: mpsc::Sender<player::Command>,
    volume: watch::Sender<player::VolumeRequest>,
    updates: watch::Receiver<player::State>,
    endpoint: native::Endpoint,
    tray_endpoint: tray::Endpoint,
    mut window_requests: mpsc::Receiver<tray::WindowAction>,
) {
    #[cfg(not(target_os = "windows"))]
    drop(endpoint);
    let (application, visibility) = window::application();
    application.with_assets(AppAssets).run(move |cx| {
        init(cx);
        style::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = window_options(cx);
        #[cfg(target_os = "windows")]
        let tray_task = tray::windows::attach(cx, tray_endpoint.clone());
        let quit_endpoint = tray_endpoint.clone();
        cx.on_app_quit(move |_| {
            quit_endpoint
                .available
                .store(false, std::sync::atomic::Ordering::Release);
            async {}
        })
        .detach();
        let close_endpoint = tray_endpoint.clone();
        let opened = open_window(options, cx, |window, cx| {
            window.on_window_should_close(cx, move |_, _| {
                if close_endpoint.hide_on_close() {
                    // Dispatch outside the close callback, avoiding reentrant native events.
                    close_endpoint
                        .window
                        .try_send(tray::WindowAction::Hide)
                        .is_err()
                } else {
                    true
                }
            });
            #[cfg(target_os = "windows")]
            let media = native::windows::attach(window, cx, &endpoint);
            cx.new(|cx| {
                let app = app::PlayerApp::new(window, cx, commands, volume, updates);
                #[cfg(target_os = "windows")]
                let app = app.with_media(media);
                app
            })
        });
        match opened {
            Ok((window, _)) => {
                cx.spawn(async move |cx| {
                    #[cfg(target_os = "windows")]
                    let _tray_task = tray_task;
                    while let Some(action) = window_requests.recv().await {
                        match action {
                            tray::WindowAction::Quit => {
                                tray_endpoint
                                    .available
                                    .store(false, std::sync::atomic::Ordering::Release);
                                window_requests.close();
                                cx.update(|cx| cx.quit());
                                break;
                            }
                            tray::WindowAction::Show => {
                                if let Err(error) = visibility(window, true, cx) {
                                    tracing::error!(%error, "Could not restore Grajek window");
                                }
                            }
                            tray::WindowAction::Hide if tray_endpoint.available() => {
                                if let Err(error) = visibility(window, false, cx) {
                                    tracing::error!(%error, "Could not hide Grajek window");
                                }
                            }
                            tray::WindowAction::Hide => {}
                        }
                    }
                })
                .detach();
            }
            Err(error) => {
                tracing::error!(%error, "Could not open native window");
                cx.quit();
            }
        }
    });
}
