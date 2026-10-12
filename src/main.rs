mod app;
mod config;
mod discord;
mod media;
mod player;

use anyhow::{Context as _, Result};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{
    AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, application, assets, init,
    open_window, px, size,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discord_player=info,warn".into()),
        )
        .init();
    let config = config::Config::load();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Could not start Tokio")?;
    let (commands, receiver) = mpsc::channel(4);
    let (states, updates) = watch::channel(player::State::default());
    let (shutdown, shutdown_rx) = watch::channel(false);
    let controller = runtime.spawn(player::run(config, receiver, states, shutdown_rx));
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(440.), px(260.)),
                cx,
            ))),
            window_min_size: Some(size(px(400.), px(260.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Discord Player".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) = open_window(options, cx, |window, cx| {
            cx.new(|cx| app::PlayerApp::new(window, cx, commands, updates))
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
