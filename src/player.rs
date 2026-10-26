mod controller;
mod state;

use anyhow::Result;
use songbird::tracks::{TrackResult, TrackState};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot, watch};

use crate::{config::Config, discord::Health, live::Live, media::Resolved, store};
use controller::Controller;
pub use state::{
    Command, ImportStatus, Phase, Preferences, RepeatMode, SeekStatus, State, VolumeRequest,
    percent_from, seek_target, volume_factor,
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
    fn seek(&self, position: Duration) -> impl Future<Output = TrackResult<Duration>> + Send;
}

/// The Discord and yt-dlp side of the controller, replaceable in tests.
pub trait Backend: Send + Sync + 'static {
    type Handle: Handle;
    fn setup(&self, _request: crate::setup::Request) -> impl Future<Output = Result<()>> + Send {
        async { anyhow::bail!("Setup unavailable") }
    }
    fn ready(&self) -> bool {
        true
    }
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
    /// Metadata only; must never join voice or open an audio stream.
    fn title(&self, url: String) -> impl Future<Output = Result<Option<String>>> + Send;
    fn playlist(
        &self,
        url: String,
    ) -> impl Future<Output = Result<crate::playlist::Playlist>> + Send;
    fn connect(&self) -> impl Future<Output = Result<()>> + Send;
    fn disconnect(&self) -> impl Future<Output = Result<()>> + Send;
    fn recover(&self) -> impl Future<Output = ()> + Send;
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

pub async fn run(
    config: Config,
    commands: mpsc::Receiver<Command>,
    volume: watch::Receiver<VolumeRequest>,
    states: watch::Sender<State>,
    shutdown: watch::Receiver<bool>,
) {
    let loaded = store::open();
    if let Some(warning) = &loaded.warning {
        tracing::warn!(%warning, "Saved state");
    }
    let Ok(http) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(15))
        .build()
    else {
        return;
    };
    let external = config.token.is_some();
    let backend = Arc::new(Live::new(http, external));
    let (persist, snapshots) = watch::channel(loaded.saved.clone());
    let saver = loaded.path.map(|path| store::spawn_saver(path, snapshots));
    let destination = loaded.saved.destination.clone();
    let notice = config.warning.clone().or(loaded.warning);
    let mut controller = Controller::new(backend, loaded.saved, notice, states, persist);
    controller
        .configure(crate::setup::Request::Restore {
            config,
            saved: destination,
        })
        .await;
    controller.run(commands, volume, shutdown).await;
    // The controller (and its snapshot sender) is gone; wait for the final write.
    if let Some(saver) = saver {
        let _ = saver.await;
    }
}
