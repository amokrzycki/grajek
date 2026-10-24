//! The real Discord/Songbird/yt-dlp implementation of the controller's [`Backend`].

use anyhow::{Context as _, Result};
use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler, TrackEvent,
    tracks::{PlayMode, Track, TrackHandle, TrackResult, TrackState},
};
use std::{sync::Arc, time::Duration};
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
    ) -> Result<(LiveHandle, watch::Receiver<Option<Outcome>>)> {
        let call = self.discord.manager.get_or_insert(self.discord.guild);
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

    async fn playlist(&self, url: String) -> Result<crate::playlist::Playlist> {
        Ok(crate::playlist::extract(url).await?)
    }

    async fn connect(&self) -> Result<()> {
        self.discord.connect().await
    }

    async fn disconnect(&self) -> Result<()> {
        self.discord.disconnect().await
    }

    async fn recover(&self) {
        self.discord.reset_incomplete_connection().await;
    }

    async fn shutdown(&self) {
        self.discord.shutdown().await;
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
