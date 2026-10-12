use anyhow::{Context, Result};
use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler, TrackEvent,
    input::Input,
    tracks::{PlayMode, TrackHandle},
};
use std::{future::pending, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::{interval, timeout},
};

use crate::{
    config::{Config, video_url},
    discord::Discord,
    media,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Initializing,
    Ready,
    Connecting,
    Resolving,
    Playing,
    Stopping,
    Error,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Initializing => "Initializing…",
            Self::Ready => "Ready",
            Self::Connecting => "Connecting…",
            Self::Resolving => "Resolving audio…",
            Self::Playing => "Playing",
            Self::Stopping => "Stopping…",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct State {
    pub phase: Phase,
    pub error: Option<String>,
    pub available: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            phase: Phase::Initializing,
            error: None,
            available: false,
        }
    }
}

impl State {
    pub fn can_play(&self) -> bool {
        self.available && matches!(self.phase, Phase::Ready | Phase::Playing | Phase::Error)
    }
    pub fn can_stop(&self) -> bool {
        self.available
            && matches!(
                self.phase,
                Phase::Connecting | Phase::Resolving | Phase::Playing | Phase::Error
            )
    }
}

pub enum Command {
    Play(String),
    Stop,
}

fn publish(states: &watch::Sender<State>, phase: Phase, error: Option<String>, available: bool) {
    tracing::info!(state = ?phase, "Player state");
    states.send_replace(State {
        phase,
        error,
        available,
    });
}

#[derive(Clone, Debug)]
enum Outcome {
    End,
    Error(String),
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

async fn cancel_job(job: &mut Option<JoinHandle<Result<Input>>>) {
    if let Some(job) = job.take() {
        job.abort();
        let _ = job.await;
    }
}

fn stop_track(track: &mut Option<TrackHandle>) {
    if let Some(track) = track.take()
        && let Err(error) = track.stop()
    {
        tracing::debug!(%error, "Track already finished");
    }
}

pub async fn run(
    config: Result<Config>,
    mut commands: mpsc::Receiver<Command>,
    states: watch::Sender<State>,
    mut shutdown: watch::Receiver<bool>,
) {
    let startup = async {
        let config = config?;
        media::check_tools().await?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(15))
            .build()
            .context("Could not create audio HTTP client.")?;
        Ok::<_, anyhow::Error>((Arc::new(Discord::start(config).await?), http))
    };
    let startup = tokio::select! {
        biased;
        _ = shutdown.changed() => return,
        result = startup => result,
    };
    let (discord, http) = match startup {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "Player initialization failed");
            publish(&states, Phase::Error, Some(error.to_string()), false);
            tokio::select! { _ = shutdown.changed() => {}, _ = commands.recv() => {} }
            return;
        }
    };
    let mut health = discord.health.clone();
    let mut job: Option<JoinHandle<Result<Input>>> = None;
    let mut track: Option<TrackHandle> = None;
    let (_, mut outcome) = watch::channel(None::<Outcome>);
    let mut tick = interval(Duration::from_millis(250));
    let mut started = std::time::Instant::now();
    publish(&states, Phase::Ready, None, true);

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            command = commands.recv() => {
                match command {
                    None => break,
                    Some(Command::Stop) => {
                        publish(&states, Phase::Stopping, None, true);
                        cancel_job(&mut job).await;
                        stop_track(&mut track);
                        discord.reset_incomplete_connection().await;
                        let connected = health.borrow().gateway;
                        let error = health.borrow().error();
                        publish(&states, if error.is_some() { Phase::Error } else { Phase::Ready }, error, connected);
                    },
                    Some(Command::Play(value)) => {
                        if job.is_some() || !health.borrow().gateway { continue; }
                        let url = match video_url(&value) {
                            Ok(url) => url,
                            Err(error) => {
                                let phase = if track.is_some() { Phase::Playing } else { Phase::Error };
                                publish(&states, phase, Some(error.to_string()), true);
                                continue;
                            },
                        };
                        stop_track(&mut track);
                        discord.clear_voice_error();
                        publish(&states, Phase::Connecting, None, true);
                        let discord = discord.clone();
                        let http = http.clone();
                        let states = states.clone();
                        job = Some(tokio::spawn(async move {
                            discord.connect().await?;
                            publish(&states, Phase::Resolving, None, true);
                            media::resolve(http, url).await
                        }));
                    },
                }
            },
            changed = health.changed() => {
                if changed.is_err() { break; }
                let current = health.borrow_and_update().clone();
                if let Some(error) = current.error() {
                    cancel_job(&mut job).await;
                    stop_track(&mut track);
                    publish(&states, Phase::Error, Some(error), current.gateway);
                } else if current.gateway && states.borrow().phase == Phase::Error && job.is_none() && track.is_none() {
                    publish(&states, Phase::Ready, None, true);
                }
            },
            result = async {
                match &mut job { Some(job) => job.await, None => pending().await }
            } => {
                job = None;
                match result.context("Audio request task failed.").and_then(|result| result) {
                    Ok(input) => {
                        let call = discord.manager.get_or_insert(discord.guild);
                        let handle = call.lock().await.play_only_input(input);
                        // Each track has its own mailbox; replaced tracks cannot update this one.
                        let (sender, receiver) = watch::channel(None);
                        outcome = receiver;
                        let registration = handle.add_event(Event::Track(TrackEvent::End), TrackEvents(sender.clone()))
                            .and_then(|_| handle.add_event(Event::Track(TrackEvent::Error), TrackEvents(sender)));
                        if let Err(error) = registration {
                            let _ = handle.stop();
                            tracing::error!(%error, "Could not observe audio track");
                            publish(&states, Phase::Error, Some("Could not start audio playback. Try again.".into()), true);
                        } else { track = Some(handle); started = std::time::Instant::now(); }
                    },
                    Err(error) => {
                        tracing::error!(error = %format!("{error:#}"), "Audio request failed");
                        discord.reset_incomplete_connection().await;
                        publish(&states, Phase::Error, Some(media::user_error(&error)), health.borrow().gateway);
                    },
                }
            },
            changed = outcome.changed(), if track.is_some() => {
                if changed.is_ok() {
                    let event = outcome.borrow_and_update().clone();
                    match event {
                        Some(Outcome::End) => { stop_track(&mut track); publish(&states, Phase::Ready, None, true); },
                        Some(Outcome::Error(error)) => { stop_track(&mut track); publish(&states, Phase::Error, Some(error), true); },
                        None => {},
                    }
                } else {
                    stop_track(&mut track);
                    publish(&states, Phase::Error, Some("Audio driver terminated unexpectedly. Try again.".into()), true);
                }
            },
            _ = tick.tick(), if track.is_some() => {
                if let Some(handle) = &track {
                    match timeout(Duration::from_secs(2), handle.get_info()).await {
                        Ok(Ok(info)) if matches!(info.playing, PlayMode::Play) && info.position > Duration::ZERO => {
                            if states.borrow().phase != Phase::Playing { publish(&states, Phase::Playing, None, true); }
                        },
                        Ok(Ok(info)) if info.playing.is_done() => {
                            let error = match info.playing { PlayMode::Errored(error) => Some(error.to_string()), _ => None };
                            stop_track(&mut track);
                            publish(&states, if error.is_some() { Phase::Error } else { Phase::Ready }, error, true);
                        },
                        Ok(Err(error)) => {
                            tracing::debug!(%error, "Track handle finished");
                            // Allow terminal events to arrive; a dead driver cannot hold the UI forever.
                            if started.elapsed() > Duration::from_secs(10) {
                                stop_track(&mut track);
                                publish(&states, Phase::Error, Some("Audio track terminated unexpectedly. Try again.".into()), true);
                            }
                        },
                        Err(_) => {
                            stop_track(&mut track);
                            publish(&states, Phase::Error, Some("Audio driver stopped responding. Try again.".into()), true);
                        },
                        _ if started.elapsed() > Duration::from_secs(10) && states.borrow().phase != Phase::Playing => {
                            stop_track(&mut track);
                            publish(&states, Phase::Error, Some("Audio playback did not start. Try another video.".into()), true);
                        },
                        _ => {},
                    }
                }
            },
        }
    }
    cancel_job(&mut job).await;
    stop_track(&mut track);
    discord.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_follow_operational_state() {
        for phase in [
            Phase::Initializing,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Stopping,
        ] {
            assert!(
                !State {
                    phase,
                    error: None,
                    available: true
                }
                .can_play()
            );
        }
        for phase in [Phase::Ready, Phase::Playing, Phase::Error] {
            assert!(
                State {
                    phase,
                    error: None,
                    available: true
                }
                .can_play()
            );
            assert!(
                !State {
                    phase,
                    error: None,
                    available: false
                }
                .can_play()
            );
        }
        assert!(
            !State {
                phase: Phase::Ready,
                error: None,
                available: true
            }
            .can_stop()
        );
        assert!(
            State {
                phase: Phase::Resolving,
                error: None,
                available: true
            }
            .can_stop()
        );
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn stop_cancels_pending_work_and_terminates_its_child() -> Result<()> {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut child = tokio::process::Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()?;
        let pid = child.id().context("test child has no process ID")?;
        let mut job = Some(tokio::spawn(async move {
            let _ = child.wait().await;
            let _ = sender.send(()).await;
            anyhow::bail!("unexpected completion")
        }));
        cancel_job(&mut job).await;
        assert!(job.is_none());
        assert_eq!(receiver.recv().await, None);
        timeout(Duration::from_secs(2), async {
            while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("cancelled request left its child running")?;
        Ok(())
    }

    #[tokio::test]
    async fn decode_error_is_reported_by_both_terminal_events() {
        let mut driver = songbird::Driver::new(songbird::Config::default());
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
}
