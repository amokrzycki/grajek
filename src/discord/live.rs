//! The real Discord/Songbird/yt-dlp implementation of the controller's [`Backend`].

use anyhow::{Context as _, Result};
use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler, TrackEvent,
    tracks::{PlayMode, Track, TrackHandle, TrackResult, TrackState},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{RwLock, oneshot, watch};

use crate::{
    discord::{Discord, Health, setup},
    media::{self, Resolved},
    player::{Backend, Handle, Outcome},
};

mod configure;

pub struct Live {
    discord: RwLock<Option<Discord>>,
    credentials: tokio::sync::Mutex<Option<(String, bool)>>,
    health: watch::Sender<Health>,
    external: bool,
    http: reqwest::Client,
}

impl Live {
    pub fn new(http: reqwest::Client, external: bool) -> Self {
        let (health, _) = watch::channel(Health::default());
        health.send_modify(|health| health.setup.external = external);
        Self {
            discord: RwLock::new(None),
            credentials: tokio::sync::Mutex::new(None),
            health,
            http,
            external,
        }
    }
}

#[derive(Clone)]
pub struct LiveHandle {
    track: TrackHandle,
    source: Option<media::SourceControl>,
}

impl Handle for LiveHandle {
    fn set_volume(&self, volume: f32) -> TrackResult<()> {
        self.track.set_volume(volume)
    }
    fn pause(&self) -> TrackResult<()> {
        self.track.pause()
    }
    fn resume(&self) -> TrackResult<()> {
        self.track.play()
    }
    fn stop(&self) -> TrackResult<()> {
        if let Some(source) = &self.source {
            source.stop();
        }
        self.track.stop()
    }
    async fn info(&self) -> TrackResult<TrackState> {
        self.track
            .get_info()
            .await
            .map(|state| source_state(state, self.source.as_ref()))
    }
    async fn seek(&self, position: Duration) -> TrackResult<Duration> {
        self.track.seek_async(position).await
    }
}

impl Backend for Live {
    type Handle = LiveHandle;

    fn ready(&self) -> bool {
        let health = self.health.borrow();
        health
            .setup
            .destination
            .as_ref()
            .is_some_and(|destination| {
                health
                    .setup
                    .servers
                    .iter()
                    .any(|server| server.id == destination.guild && server.available)
            })
    }

    fn health(&self) -> watch::Receiver<Health> {
        self.health.subscribe()
    }

    fn clear_error(&self) {
        self.health.send_modify(|health| {
            health.voice_error = None;
            health.voice_retry = false;
        });
    }

    async fn setup(&self, request: setup::Request) -> Result<()> {
        if self.external && matches!(request, setup::Request::Authenticate { .. }) {
            anyhow::bail!("Token is externally managed. Change it in your environment.");
        }
        let request = if matches!(request, setup::Request::Retry) {
            let (token, remember) = self
                .credentials
                .lock()
                .await
                .clone()
                .context("Enter a bot token first.")?;
            setup::Request::Authenticate { token, remember }
        } else {
            request
        };
        let previous = self.health.borrow().setup.destination.clone();
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            match request {
                setup::Request::Restore { config, saved } => self.restore(config, saved).await,
                request @ setup::Request::Authenticate { .. } => {
                    self.configure(request).await?;
                    self.configure(setup::Request::Refresh).await?;
                    let bot = self.health.borrow().setup.bot;
                    if let Some(destination) =
                        previous.filter(|destination| Some(destination.bot) == bot)
                    {
                        self.configure(setup::Request::Select(destination)).await?;
                    }
                    Ok(())
                }
                request => self.configure(request).await,
            }
        })
        .await
        .context("Discord setup timed out. Check the network and retry.")
        .and_then(|result| result);
        if let Err(error) = &result {
            self.health.send_modify(|health| {
                health.setup.status = setup::Status::Failed(error.to_string());
            });
        }
        result
    }

    async fn check_tools(&self) -> Result<()> {
        media::check_tools().await
    }

    async fn prepare(&self, url: String, connected: oneshot::Sender<()>) -> Result<Resolved> {
        self.connect().await?;
        let _ = connected.send(());
        media::resolve(self.http.clone(), url).await
    }

    async fn play(
        &self,
        resolved: Resolved,
        volume: f32,
    ) -> Result<(LiveHandle, watch::Receiver<Option<Outcome>>)> {
        let session = self.discord.read().await;
        let discord = session.as_ref().context("Configure a bot first.")?;
        let call = discord
            .manager
            .get_or_insert(discord.guild.context("Select a voice channel first.")?);
        drop(session);
        // The volume is part of the track, so even its first frame uses it.
        let handle = call
            .lock()
            .await
            .play_only(Track::from(resolved.input).volume(volume));
        let (sender, outcome) = watch::channel(None);
        let registration = handle
            .add_event(
                Event::Track(TrackEvent::End),
                TrackEvents(sender.clone(), resolved.source.clone()),
            )
            .and_then(|()| {
                handle.add_event(
                    Event::Track(TrackEvent::Error),
                    TrackEvents(sender, resolved.source.clone()),
                )
            });
        if let Err(error) = registration {
            if let Some(source) = &resolved.source {
                source.stop();
            }
            let _ = handle.stop();
            return Err(error).context("Could not observe audio track");
        }
        Ok((
            LiveHandle {
                track: handle,
                source: resolved.source,
            },
            outcome,
        ))
    }

    async fn title(&self, url: String) -> Result<Option<String>> {
        media::title(url).await
    }

    async fn search(
        &self,
        query: String,
        cancel: watch::Receiver<bool>,
    ) -> std::result::Result<crate::search::Results, crate::search::Error> {
        crate::search::extract(query, cancel).await
    }
    async fn playlist(&self, url: String) -> Result<crate::playlist::Playlist> {
        Ok(crate::playlist::extract(url).await?)
    }

    async fn connect(&self) -> Result<()> {
        self.discord
            .read()
            .await
            .as_ref()
            .context("Configure a bot first.")?
            .connect()
            .await
    }

    async fn disconnect(&self) -> Result<()> {
        let session = self.discord.read().await;
        match session.as_ref() {
            Some(discord) => discord.disconnect().await,
            None => Ok(()),
        }
    }

    async fn recover(&self) {
        if let Some(discord) = self.discord.read().await.as_ref() {
            discord.reset_incomplete_connection().await;
        }
    }

    async fn shutdown(&self) {
        let discord = self.discord.write().await.take();
        if let Some(discord) = discord {
            discord.shutdown().await;
        }
    }
}

/// Songbird 0.6.1 maps packet-read errors to End. Preserve range failures in both
/// event and info paths so they never count as successful natural completion.
pub(crate) fn source_state(
    mut state: TrackState,
    source: Option<&media::SourceControl>,
) -> TrackState {
    if state.playing == PlayMode::End && source.is_some_and(media::SourceControl::failed) {
        state.playing = PlayMode::Errored(songbird::tracks::PlayError::Decode(Arc::new(
            symphonia::core::errors::Error::IoError(std::io::Error::other(
                "Audio stream read failed. Check media access and the network.",
            )),
        )));
    }
    state
}

struct TrackEvents(watch::Sender<Option<Outcome>>, Option<media::SourceControl>);

#[async_trait]
impl EventHandler for TrackEvents {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(tracks) = context {
            for (state, _) in *tracks {
                let state = source_state((*state).clone(), self.1.as_ref());
                let outcome = match &state.playing {
                    PlayMode::Errored(error) => {
                        tracing::error!(%error, "Audio track failed");
                        Outcome::Error(
                            "Audio playback failed. Check media access and the logs.".into(),
                        )
                    }
                    PlayMode::End => Outcome::End,
                    _ => Outcome::Error(
                        "Audio track terminated unexpectedly. Press Next or Play URL.".into(),
                    ),
                };
                // Error and End may both arrive; a late End must never erase a failure.
                self.0.send_if_modified(|current| {
                    if matches!(current, Some(Outcome::Error(_))) {
                        return false;
                    }
                    *current = Some(outcome);
                    true
                });
            }
        }
        Some(Event::Cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::TrackEvents;
    use crate::player::{Outcome, volume_factor};
    use songbird::{
        Driver, EventContext, EventHandler,
        tracks::{PlayMode, Track},
    };
    use std::sync::Arc;
    use tokio::sync::watch;

    #[tokio::test]
    async fn decode_error_is_reported_by_both_terminal_events() {
        let mut driver = Driver::new(songbird::Config::default());
        let handle = driver.play_only_input(Vec::<u8>::new().into());
        let (sender, receiver) = watch::channel(None);
        let handler = TrackEvents(sender, None);
        let state = songbird::tracks::TrackState {
            playing: PlayMode::Errored(songbird::tracks::PlayError::Decode(Arc::new(
                symphonia::core::errors::Error::DecodeError("invalid packet"),
            ))),
            ..Default::default()
        };
        let tracks = [(&state, &handle)];
        // Songbird emits Error and End for the same failure; neither may erase it.
        handler.act(&EventContext::Track(&tracks)).await;
        handler.act(&EventContext::Track(&tracks)).await;
        assert!(
            matches!(&*receiver.borrow(), Some(Outcome::Error(message)) if message.contains("playback failed"))
        );
        let ended = songbird::tracks::TrackState {
            playing: PlayMode::End,
            ..Default::default()
        };
        handler
            .act(&EventContext::Track(&[(&ended, &handle)]))
            .await;
        assert!(matches!(&*receiver.borrow(), Some(Outcome::Error(_))));
        driver.stop();
    }

    #[tokio::test]
    async fn range_failure_cannot_be_natural_completion_in_events_or_polling() {
        let source = crate::media::SourceControl::default();
        let ended = songbird::tracks::TrackState {
            playing: PlayMode::End,
            ..Default::default()
        };
        assert_eq!(
            super::source_state(ended.clone(), Some(&source)).playing,
            PlayMode::End
        );
        source.record_failure();
        assert!(matches!(
            super::source_state(ended.clone(), Some(&source)).playing,
            PlayMode::Errored(_)
        ));
        let playing = songbird::tracks::TrackState::default();
        assert_eq!(
            super::source_state(playing, Some(&source)).playing,
            PlayMode::Play,
            "source signal alone must not fabricate a terminal event"
        );
        let mut driver = Driver::new(songbird::Config::default());
        let handle = driver.play_only_input(Vec::<u8>::new().into());
        let (sender, receiver) = watch::channel(None);
        let handler = TrackEvents(sender, Some(source));
        handler
            .act(&EventContext::Track(&[(&ended, &handle)]))
            .await;
        assert!(matches!(&*receiver.borrow(), Some(Outcome::Error(_))));
        driver.stop();
    }

    #[test]
    fn new_tracks_start_at_the_selected_volume() {
        for percent in [0, 1, 40, 100] {
            let track = Track::from(Vec::<u8>::new()).volume(volume_factor(percent));
            assert!((track.volume - volume_factor(percent)).abs() < f32::EPSILON);
        }
    }
}
