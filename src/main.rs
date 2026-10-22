mod app;
mod config;
mod discord;
mod live;
mod media;
mod player;
mod playlist;
mod queue;
mod store;
mod style;

use anyhow::{Context as _, Result};
use gpui_kit::{
    AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, application, assets, init,
    open_window, px, size,
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
    let controller = runtime.spawn(player::run(
        config,
        receiver,
        volume_rx,
        states,
        shutdown_rx,
    ));
    application().with_assets(AppAssets).run(move |cx| {
        init(cx);
        style::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
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
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| app::PlayerApp::new(window, cx, commands, volume, updates))
        }) {
            tracing::error!(%error, "Could not open native window");
            cx.quit();
        }
    });
    shutdown.send_replace(true);
    runtime.block_on(async {
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
