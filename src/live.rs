//! The real Discord/Songbird/yt-dlp implementation of the controller's [`Backend`].

use anyhow::{Context as _, Result};
use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler, TrackEvent,
    tracks::{PlayMode, Track, TrackHandle, TrackResult, TrackState},
};
use std::sync::Arc;
use tokio::sync::{oneshot, watch};

use crate::{
    discord::{Discord, Health},
    media::{self, Resolved},
    player::{Backend, Handle, Outcome},
};

pub struct Live {
    discord: Arc<Discord>,
    http: reqwest::Client,
}

impl Live {
    pub const fn new(discord: Arc<Discord>, http: reqwest::Client) -> Self {
        Self { discord, http }
    }
}

impl Handle for TrackHandle {
    fn set_volume(&self, volume: f32) -> TrackResult<()> {
        Self::set_volume(self, volume)
    }
    fn pause(&self) -> TrackResult<()> {
        Self::pause(self)
    }
    fn resume(&self) -> TrackResult<()> {
        Self::play(self)
    }
    fn stop(&self) -> TrackResult<()> {
        Self::stop(self)
    }
    async fn info(&self) -> TrackResult<TrackState> {
        self.get_info().await
    }
}

impl Backend for Live {
    type Handle = TrackHandle;

    fn health(&self) -> watch::Receiver<Health> {
        self.discord.health.clone()
    }

    fn clear_error(&self) {
        self.discord.clear_voice_error();
    }

    async fn prepare(&self, url: String, connected: oneshot::Sender<()>) -> Result<Resolved> {
        self.discord.connect().await?;
        let _ = connected.send(());
        media::resolve(self.http.clone(), url).await
    }

    async fn play(
        &self,
        resolved: Resolved,
        volume: f32,
    ) -> Result<(TrackHandle, watch::Receiver<Option<Outcome>>)> {
        let call = self.discord.manager.get_or_insert(self.discord.guild);
        // The volume is part of the track, so even its first frame uses it.
        let handle = call
            .lock()
            .await
            .play_only(Track::from(resolved.input).volume(volume));
        let (sender, outcome) = watch::channel(None);
        let registration = handle
            .add_event(Event::Track(TrackEvent::End), TrackEvents(sender.clone()))
            .and_then(|()| handle.add_event(Event::Track(TrackEvent::Error), TrackEvents(sender)));
        if let Err(error) = registration {
            let _ = handle.stop();
            return Err(error).context("Could not observe audio track");
        }
        Ok((handle, outcome))
    }

    async fn recover(&self) {
        self.discord.reset_incomplete_connection().await;
    }

    async fn shutdown(&self) {
        self.discord.shutdown().await;
    }
}

struct TrackEvents(watch::Sender<Option<Outcome>>);

#[async_trait]
impl EventHandler for TrackEvents {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(tracks) = context {
            for (state, _) in *tracks {
                let outcome = match &state.playing {
                    PlayMode::Errored(error) => {
                        tracing::error!(%error, "Audio track failed");
                        Outcome::Error(
                            "Audio playback failed. Check media access and the logs.".into(),
                        )
                    }
                    _ => Outcome::End,
                };
                self.0.send_replace(Some(outcome));
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
        let handler = TrackEvents(sender);
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
