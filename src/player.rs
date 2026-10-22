mod controller;
mod state;

use anyhow::{Context, Result};
use songbird::tracks::{TrackResult, TrackState};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    config::Config,
    discord::{Discord, Health},
    live::Live,
    media::{self, Resolved},
    store,
};
use controller::Controller;
pub use state::{
    Command, Phase, Preferences, RepeatMode, State, VolumeRequest, percent_from, volume_factor,
};

/// How a track ended, as reported by its own event mailbox.
#[derive(Clone, Debug)]
pub enum Outcome {
    End,
    Error(String),
}

/// What the controller needs from a playing track.
pub trait Handle: Clone + Send + Sync + 'static {
    fn set_volume(&self, volume: f32) -> TrackResult<()>;
    fn pause(&self) -> TrackResult<()>;
    fn resume(&self) -> TrackResult<()>;
    fn stop(&self) -> TrackResult<()>;
    fn info(&self) -> impl Future<Output = TrackResult<TrackState>> + Send;
}

/// The Discord and yt-dlp side of the controller, replaceable in tests.
pub trait Backend: Send + Sync + 'static {
    type Handle: Handle;
    fn health(&self) -> watch::Receiver<Health>;
    fn clear_error(&self);
    /// Joins voice, then extracts the audio. `connected` fires once voice is ready.
    fn prepare(
        &self,
        url: String,
        connected: oneshot::Sender<()>,
    ) -> impl Future<Output = Result<Resolved>> + Send;
    /// Starts the track at `volume` and returns its handle and its own event mailbox.
    fn play(
        &self,
        resolved: Resolved,
        volume: f32,
    ) -> impl Future<Output = Result<(Self::Handle, watch::Receiver<Option<Outcome>>)>> + Send;
    fn connect(&self) -> impl Future<Output = Result<()>> + Send;
    fn disconnect(&self) -> impl Future<Output = Result<()>> + Send;
    fn recover(&self) -> impl Future<Output = ()> + Send;
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

pub async fn run(
    config: Result<Config>,
    commands: mpsc::Receiver<Command>,
    volume: watch::Receiver<VolumeRequest>,
    states: watch::Sender<State>,
    mut shutdown: watch::Receiver<bool>,
) {
    let loaded = store::open();
    if let Some(warning) = &loaded.warning {
        tracing::warn!(%warning, "Saved state");
    }
    let startup = async {
        let config = config?;
        media::check_tools().await?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()
            .context("Could not create audio HTTP client.")?;
        Ok::<_, anyhow::Error>(Live::new(Arc::new(Discord::start(config).await?), http))
    };
    let startup = tokio::select! {
        biased;
        _ = shutdown.changed() => return,
        result = startup => result,
    };
    let backend = match startup {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "Player initialization failed");
            states.send_replace(State {
                phase: Phase::Error,
                error: Some(error.to_string()),
                volume: loaded.saved.volume,
                preferences: Preferences {
                    continue_on_error: loaded.saved.continue_on_error,
                    remember_queue: loaded.saved.remember_queue,
                    repeat: loaded.saved.repeat,
                },
                queue: loaded.saved.restored_queue().snapshot(),
                history: loaded.saved.history,
                notice: loaded.warning,
                ..State::default()
            });
            let mut commands = commands;
            tokio::select! { _ = shutdown.changed() => {}, _ = commands.recv() => {} }
            return;
        }
    };
    let (persist, snapshots) = watch::channel(loaded.saved.clone());
    let saver = loaded.path.map(|path| store::spawn_saver(path, snapshots));
    let controller = Controller::new(backend, loaded.saved, loaded.warning, states, persist);
    controller.run(commands, volume, shutdown).await;
    // The controller (and its snapshot sender) is gone; wait for the final write.
    if let Some(saver) = saver {
        let _ = saver.await;
    }
}
