#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod config;
mod destination;
mod discord;
mod favorites;
mod history;
mod identity;
mod media;
mod paths;
mod platform;
mod player;
mod playlist;
mod preferences;
mod queue;
mod search;
mod store;
mod style;
#[cfg(test)]
mod test_support;
mod youtube;
mod ytdlp;

use anyhow::{Context as _, Result};
use gpui_kit::{
    AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, assets, init, open_window,
    px, size,
};
use std::{borrow::Cow, time::Duration};
use tokio::sync::{mpsc, watch};

// Extend the default Lucide assets with the speaker and transport icons we use.
gpui_kit::assets::icon_assets!(
    ControlIcons,
    [
        Volume2,
        VolumeX,
        Pause,
        SkipForward,
        Square,
        ListPlus,
        ListStart,
        CornerDownLeft,
        Import,
        Check
    ]
);

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
    #[cfg(target_os = "windows")]
    let _app_mutex =
        grajek_smtc::hold_app_mutex().context("Could not create the Grajek installer mutex.")?;
    let tools_dir = paths::tools_dir();
    if let Some(dir) = &tools_dir
        && let Err(error) = media::components::download::recover(dir)
    {
        tracing::warn!(%error, "Could not recover playback components");
    }
    #[cfg(target_os = "windows")]
    let bundled_tools = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("tools")));
    #[cfg(target_os = "windows")]
    media::components::initialize_with_fallback(tools_dir, bundled_tools);
    #[cfg(target_os = "linux")]
    media::components::initialize(tools_dir);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Could not start Tokio")?;
    let (commands, receiver) = mpsc::channel(16);
    let (volume, volume_rx) = watch::channel(player::VolumeRequest::default());
    let (states, updates) = watch::channel(player::State::default());
    let (shutdown, shutdown_rx) = watch::channel(false);
    let (window_commands, window_requests) = mpsc::channel(16);
    let tray_endpoint = platform::tray::Endpoint {
        player: platform::media::Endpoint {
            commands: commands.clone(),
            states: updates.clone(),
        },
        window: window_commands,
        available: platform::tray::TrayAvailability::default(),
    };
    #[cfg(target_os = "linux")]
    let tray_session = runtime.spawn(platform::tray::linux::run(
        tray_endpoint.clone(),
        shutdown_rx.clone(),
    ));
    #[cfg(target_os = "linux")]
    let media_session = runtime.spawn(platform::media::linux::run(
        tray_endpoint.player.clone(),
        shutdown_rx.clone(),
    ));
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
        tray_endpoint,
        window_requests,
        runtime.handle().clone(),
    );
    shutdown.send_replace(true);
    runtime.block_on(async {
        #[cfg(target_os = "linux")]
        {
            await_or_abort(tray_session).await;
            await_or_abort(media_session).await;
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

#[cfg(target_os = "linux")]
async fn await_or_abort(mut session: tokio::task::JoinHandle<()>) {
    if tokio::time::timeout(Duration::from_secs(2), &mut session)
        .await
        .is_err()
    {
        session.abort();
    }
}

fn window_options(cx: &gpui_kit::App) -> WindowOptions {
    WindowOptions {
        app_id: Some(identity::APP_ID.into()),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(520.), px(560.)),
            cx,
        ))),
        window_min_size: Some(size(
            app::fmt::WINDOW_MIN_WIDTH,
            app::fmt::WINDOW_MIN_HEIGHT,
        )),
        titlebar: Some(TitlebarOptions {
            title: Some(identity::APP_NAME.into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn run_app(
    commands: mpsc::Sender<player::Command>,
    volume: watch::Sender<player::VolumeRequest>,
    updates: watch::Receiver<player::State>,
    tray_endpoint: platform::tray::Endpoint,
    mut window_requests: mpsc::Receiver<platform::tray::WindowAction>,
    runtime: tokio::runtime::Handle,
) {
    #[cfg(target_os = "windows")]
    let media_endpoint = tray_endpoint.player.clone();
    let (application, visibility) = platform::window::application();
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
        let tray_task = platform::tray::windows::attach(cx, tray_endpoint.clone());
        let quit_endpoint = tray_endpoint.clone();
        cx.on_app_quit(move |_| {
            quit_endpoint.available.mark_unavailable();
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
                        .try_send(platform::tray::WindowAction::Hide)
                        .is_err()
                } else {
                    true
                }
            });
            #[cfg(target_os = "windows")]
            let media = platform::media::windows::attach(window, cx, &media_endpoint);
            cx.new(|cx| {
                let app = app::PlayerApp::new(window, cx, commands, volume, updates);
                let app = app.with_components(runtime);
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
                            platform::tray::WindowAction::Quit => {
                                tray_endpoint.available.mark_unavailable();
                                window_requests.close();
                                cx.update(|cx| cx.quit());
                                break;
                            }
                            platform::tray::WindowAction::Show => {
                                if let Err(error) = visibility(window, true, cx) {
                                    tracing::error!(%error, "Could not restore Grajek window");
                                }
                            }
                            platform::tray::WindowAction::Hide if tray_endpoint.available() => {
                                if let Err(error) = visibility(window, false, cx) {
                                    tracing::error!(%error, "Could not hide Grajek window");
                                }
                            }
                            platform::tray::WindowAction::Hide => {}
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
