use anyhow::{Context as _, Result, ensure};
use serenity::{
    all::{
        ChannelId, ChannelType, Client, ConnectionStage, Context, EventHandler, GatewayIntents,
        GuildId, Http, Permissions, Ready, ShardManager, ShardStageUpdateEvent, UserId, VoiceState,
    },
    async_trait,
};
use songbird::{
    CoreEvent, Event, EventContext, EventHandler as VoiceEventHandler, SerenityInit, Songbird,
    error::JoinError, events::context_data::DisconnectReason, model::CloseCode,
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::timeout};

use crate::config::Config;

#[derive(Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "gateway, voice, join intent and retry eligibility are independent facts"
)]
pub struct Health {
    pub gateway: bool,
    pub voice: bool,
    pub gateway_error: Option<String>,
    pub voice_error: Option<String>,
    /// Only transient failures remaining after Songbird's own retries may rejoin.
    pub voice_retry: bool,
    pub voice_epoch: u64,
    pub voice_requested: bool,
    // Session IDs are compared for stale-event protection, never displayed or logged.
    pub voice_session: Option<String>,
}

impl Health {
    fn voice_state(&mut self, session: &str, in_channel: bool) -> bool {
        if !self.voice_requested {
            return false;
        }
        if in_channel {
            if self.voice_session.is_some() {
                return false;
            }
            // The gateway identifies the session before the driver finishes connecting.
            self.voice_session = Some(session.into());
            return true;
        }
        if self.voice_session.as_deref() != Some(session) {
            return false;
        }
        self.voice = false;
        self.voice_retry = false;
        self.voice_requested = false;
        self.voice_epoch = self.voice_epoch.wrapping_add(1);
        self.voice_error = Some("Bot left or was moved from the configured voice channel. Check permissions, then press Next or Play URL.".into());
        true
    }

    pub fn error(&self) -> Option<String> {
        self.gateway_error
            .clone()
            .or_else(|| self.voice_error.clone())
    }
}

struct GatewayEvents {
    ready: watch::Sender<bool>,
    health: watch::Sender<Health>,
    guild: GuildId,
    channel: ChannelId,
    user: UserId,
}

#[async_trait]
impl EventHandler for GatewayEvents {
    async fn ready(&self, _: Context, _: Ready) {
        self.health.send_modify(|health| {
            health.gateway = true;
            health.gateway_error = None;
        });
        self.ready.send_replace(true);
        tracing::info!("Discord gateway ready");
    }

    async fn shard_stage_update(&self, _: Context, event: ShardStageUpdateEvent) {
        let connected = event.new == ConnectionStage::Connected;
        self.health.send_modify(|health| {
            health.gateway = connected;
            health.gateway_error = if connected {
                None
            } else {
                Some("Discord gateway reconnecting. Check the network.".into())
            };
        });
    }

    async fn voice_state_update(&self, _: Context, _: Option<VoiceState>, new: VoiceState) {
        if new.guild_id == Some(self.guild) && new.user_id == self.user {
            self.health.send_if_modified(|health| {
                health.voice_state(&new.session_id, new.channel_id == Some(self.channel))
            });
        }
    }
}

fn retryable(reason: Option<DisconnectReason>) -> bool {
    matches!(
        reason,
        Some(
            DisconnectReason::Io
                | DisconnectReason::TimedOut
                | DisconnectReason::WsClosed(Some(
                    CloseCode::SessionInvalid
                        | CloseCode::SessionTimeout
                        | CloseCode::VoiceServerCrash
                ))
        )
    )
}

pub(crate) fn retryable_join(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<JoinError>(),
        Some(JoinError::Driver(error)) if retryable(Some(DisconnectReason::from(error)))
    )
}

struct VoiceEvents {
    health: watch::Sender<Health>,
    epoch: u64,
}

impl VoiceEvents {
    fn update(
        &self,
        session: &str,
        connected: bool,
        reconnected: bool,
        reason: Option<DisconnectReason>,
    ) {
        self.health.send_if_modified(|health| {
            if health.voice_epoch != self.epoch || !health.voice_requested {
                return false;
            }
            if !connected && health.voice_session.as_deref().is_some_and(|current| current != session) {
                return false;
            }
            // Requested/discarded callbacks can belong to a superseded driver attempt.
            if !connected && matches!(reason, None | Some(DisconnectReason::Requested | DisconnectReason::AttemptDiscarded)) {
                return false;
            }
            health.voice_session = Some(session.into());
            health.voice = connected;
            health.voice_retry = !connected && retryable(reason);
            health.voice_error = if reconnected {
                Some("Discord voice restored. Playback was interrupted; press Next or Play URL.".into())
            } else if connected {
                None
            } else {
                Some("Discord voice disconnected. Check the network and permissions; press Next or Play URL.".into())
            };
            true
        });
    }
}

#[async_trait]
impl VoiceEventHandler for VoiceEvents {
    async fn act(&self, event: &EventContext<'_>) -> Option<Event> {
        match event {
            EventContext::DriverConnect(data) => self.update(data.session_id, true, false, None),
            EventContext::DriverReconnect(data) => self.update(data.session_id, true, true, None),
            EventContext::DriverDisconnect(data) => {
                // Never log the voice session ID or the entire event context.
                tracing::warn!(kind = ?data.kind, reason = ?data.reason, "Discord voice disconnected");
                self.update(data.session_id, false, false, data.reason);
            }
            _ => {}
        }
        None
    }
}

pub struct Discord {
    pub manager: Arc<Songbird>,
    pub guild: GuildId,
    channel: ChannelId,
    user: UserId,
    http: Arc<Http>,
    pub health: watch::Receiver<Health>,
    health_tx: watch::Sender<Health>,
    shards: Arc<ShardManager>,
    gateway: JoinHandle<()>,
}

impl Discord {
    pub async fn start(config: Config) -> Result<Self> {
        let http = Arc::new(Http::new(&config.token));
        let user = timeout(Duration::from_secs(15), http.get_current_user())
            .await
            .context("Discord authentication timed out. Check the network.")?
            .context("Discord rejected the bot token. Check DISCORD_BOT_TOKEN.")?;
        ensure!(user.bot, "Use a Discord bot token, never a user token.");
        let (health_tx, health) = watch::channel(Health::default());
        let (ready_tx, mut ready) = watch::channel(false);
        let manager = Songbird::serenity();
        let mut client = timeout(Duration::from_secs(15), async {
            Client::builder(
                &config.token,
                GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES,
            )
            .event_handler(GatewayEvents {
                ready: ready_tx,
                health: health_tx.clone(),
                guild: config.guild,
                channel: config.channel,
                user: user.id,
            })
            .register_songbird_with(manager.clone())
            .await
        })
        .await
        .context("Discord gateway setup timed out. Check the network.")?
        .context("Could not initialize the Discord gateway.")?;
        let shards = client.shard_manager.clone();
        let gateway_health = health_tx.clone();
        let gateway = tokio::spawn(async move {
            if let Err(error) = client.start().await {
                tracing::error!(%error, "Discord gateway failed");
                gateway_health.send_modify(|health| {
                    health.gateway = false;
                    health.gateway_error = Some(
                        "Discord gateway failed. Check the token/network, then restart.".into(),
                    );
                });
            }
        });
        let discord = Self {
            manager,
            guild: config.guild,
            channel: config.channel,
            user: user.id,
            http,
            health,
            health_tx,
            shards,
            gateway,
        };
        let startup = timeout(Duration::from_secs(30), async {
            while !*ready.borrow_and_update() {
                ready
                    .changed()
                    .await
                    .context("Discord gateway closed before becoming ready.")?;
            }
            discord.check_channel().await
        })
        .await
        .context("Discord initialization timed out. Check the token, guild and network.")
        .and_then(|result| result);
        if let Err(error) = startup {
            discord.shutdown().await;
            return Err(error);
        }
        Ok(discord)
    }

    async fn check_channel(&self) -> Result<()> {
        timeout(Duration::from_secs(15), async {
            let guild = self.guild.to_partial_guild(&self.http).await
                .context("Cannot access the guild. Check DISCORD_GUILD_ID and invite the bot.")?;
            let member = self.guild.member(&self.http, self.user).await
                .context("The bot is not a member of the configured guild.")?;
            let channel = self.channel.to_channel(&self.http).await
                .context("Cannot access the channel. Check DISCORD_VOICE_CHANNEL_ID and View Channel permission.")?
                .guild().context("The configured channel must belong to a guild.")?;
            ensure!(channel.guild_id == self.guild && channel.kind == ChannelType::Voice, "Choose an ordinary voice channel in the configured guild (not a stage channel).");
            let permissions = guild.user_permissions_in(&channel, &member);
            ensure!(permissions.contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK), "Bot needs View Channel, Connect and Speak in the voice channel. Check channel overrides.");
            Ok(())
        }).await.context("Checking Discord voice permissions timed out.")?
    }

    pub fn clear_voice_error(&self) {
        self.health_tx.send_modify(|health| {
            health.voice_error = None;
            health.voice_retry = false;
        });
    }

    pub async fn connect(&self) -> Result<()> {
        let call = self.manager.get_or_insert(self.guild);
        let already_joined =
            call.lock().await.current_channel().map(|id| id.0.get()) == Some(self.channel.get());
        if already_joined && self.health.borrow().voice {
            return Ok(());
        }
        drop(call);
        self.check_channel().await?;
        // A failed driver may retain its gateway channel. Remove it before a fresh join.
        self.disconnect().await?;
        let epoch = {
            let mut epoch = 0;
            self.health_tx.send_modify(|health| {
                health.voice_requested = true;
                epoch = health.voice_epoch;
            });
            epoch
        };
        let call = self.manager.get_or_insert(self.guild);
        {
            let mut call = call.lock().await;
            call.remove_all_global_events();
            for event in [
                CoreEvent::DriverConnect,
                CoreEvent::DriverReconnect,
                CoreEvent::DriverDisconnect,
            ] {
                call.add_global_event(
                    Event::Core(event),
                    VoiceEvents {
                        health: self.health_tx.clone(),
                        epoch,
                    },
                );
            }
        }
        timeout(
            Duration::from_secs(20),
            self.manager.join(self.guild, self.channel),
        )
        .await
        .context("Voice connection timed out. Check UDP/network access and permissions.")?
        .context(
            "Cannot join Discord voice. Check Connect/Speak permissions and channel capacity.",
        )?;
        self.health_tx.send_if_modified(|health| {
            if health.voice_epoch != epoch
                || !health.voice_requested
                || health.voice_error.is_some()
            {
                return false;
            }
            health.voice = true;
            true
        });
        ensure!(
            self.health.borrow().voice,
            "Discord voice connection was interrupted. Press Next or Play URL."
        );
        tracing::info!("Connected to Discord voice with Songbird's DAVE-capable driver");
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<()> {
        // Invalidate callbacks before removing the call, including its late gateway leave.
        self.health_tx.send_modify(|health| {
            health.voice_epoch = health.voice_epoch.wrapping_add(1);
            health.voice_requested = false;
            health.voice_session = None;
            health.voice = false;
            health.voice_retry = false;
            health.voice_error = None;
        });
        if self.manager.get(self.guild).is_none() {
            return Ok(());
        }
        timeout(Duration::from_secs(3), self.manager.remove(self.guild))
            .await
            .context("Leaving Discord voice timed out. Press Play URL to reconnect.")?
            .context("Could not leave Discord voice. Check the network.")?;
        Ok(())
    }

    pub async fn reset_incomplete_connection(&self) {
        if !self.health.borrow().voice {
            let _ = self.disconnect().await;
        }
    }

    pub async fn shutdown(&self) {
        if let Some(call) = self.manager.get(self.guild) {
            call.lock().await.stop();
        }
        let _ = self.disconnect().await;
        let _ = timeout(Duration::from_secs(3), self.shards.shutdown_all()).await;
        self.gateway.abort();
        tracing::info!("Discord shutdown complete");
    }
}

impl Drop for Discord {
    fn drop(&mut self) {
        self.gateway.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events() -> (VoiceEvents, watch::Receiver<Health>) {
        let (health, receiver) = watch::channel(Health {
            gateway: true,
            voice: true,
            voice_epoch: 2,
            voice_requested: true,
            voice_session: Some("current".into()),
            ..Health::default()
        });
        (VoiceEvents { health, epoch: 2 }, receiver)
    }

    #[test]
    fn only_transient_disconnect_reasons_allow_rejoin() {
        for reason in [
            DisconnectReason::Io,
            DisconnectReason::TimedOut,
            DisconnectReason::WsClosed(Some(CloseCode::SessionInvalid)),
            DisconnectReason::WsClosed(Some(CloseCode::SessionTimeout)),
            DisconnectReason::WsClosed(Some(CloseCode::VoiceServerCrash)),
        ] {
            assert!(retryable(Some(reason)));
        }
        for reason in [
            None,
            Some(DisconnectReason::Requested),
            Some(DisconnectReason::AttemptDiscarded),
            Some(DisconnectReason::Internal),
            Some(DisconnectReason::ProtocolViolation),
            Some(DisconnectReason::WsClosed(None)),
            Some(DisconnectReason::WsClosed(Some(CloseCode::Disconnected))),
            Some(DisconnectReason::WsClosed(Some(
                CloseCode::AuthenticationFailed,
            ))),
            Some(DisconnectReason::WsClosed(Some(CloseCode::RateLimited))),
            Some(DisconnectReason::WsClosed(Some(CloseCode::CallTerminated))),
        ] {
            assert!(!retryable(reason));
        }
    }

    #[test]
    fn stale_and_requested_driver_events_cannot_damage_a_new_connection() {
        let (events, health) = events();
        events.update("old", false, false, Some(DisconnectReason::Io));
        assert!(health.borrow().voice);
        for reason in [
            None,
            Some(DisconnectReason::Requested),
            Some(DisconnectReason::AttemptDiscarded),
        ] {
            events.update("current", false, false, reason);
            assert!(health.borrow().voice && health.borrow().error().is_none());
        }
        let old = VoiceEvents {
            health: events.health,
            epoch: 1,
        };
        old.update("old", false, false, Some(DisconnectReason::Io));
        old.update("old", true, false, None);
        assert_eq!(health.borrow().voice_session.as_deref(), Some("current"));
        assert!(health.borrow().voice && health.borrow().error().is_none());
    }

    #[test]
    fn intentional_disconnect_ignores_late_driver_callbacks() {
        let (events, health) = events();
        events.health.send_modify(|health| {
            health.voice_requested = false;
            health.voice = false;
        });
        events.update("current", false, false, Some(DisconnectReason::Io));
        events.update("current", true, true, None);
        assert!(!health.borrow().voice);
        assert!(health.borrow().error().is_none());
        assert!(!health.borrow().voice_retry);
    }

    #[test]
    fn internal_reconnect_reports_interruption_without_requesting_another_join() {
        let (events, health) = events();
        events.update("current", true, true, None);
        assert!(health.borrow().voice);
        assert!(!health.borrow().voice_retry);
        assert!(
            health
                .borrow()
                .error()
                .is_some_and(|e| e.contains("interrupted"))
        );
    }
    #[test]
    fn a_kick_cannot_be_overridden_by_a_late_driver_event() {
        let (events, health) = events();
        events
            .health
            .send_if_modified(|health| health.voice_state("current", false));
        events.update("current", false, false, Some(DisconnectReason::Io));
        events.update("current", true, true, None);
        assert!(!health.borrow().voice && !health.borrow().voice_retry);
        assert!(
            health
                .borrow()
                .error()
                .is_some_and(|error| error.contains("left or was moved"))
        );
        // A gateway leave for an older session must not affect a newer connection.
        let (events, health) = self::events();
        events
            .health
            .send_if_modified(|health| health.voice_state("old", false));
        assert!(health.borrow().voice && health.borrow().error().is_none());
    }

    #[test]
    fn a_kick_during_join_invalidates_a_late_driver_connect() {
        let (sender, health) = watch::channel(Health {
            gateway: true,
            voice_requested: true,
            voice_epoch: 2,
            ..Health::default()
        });
        let events = VoiceEvents {
            health: sender.clone(),
            epoch: 2,
        };
        sender.send_if_modified(|health| health.voice_state("current", true));
        assert!(!health.borrow().voice);
        // A delayed leave for the previous session must not cancel this join.
        sender.send_if_modified(|health| health.voice_state("old", false));
        assert!(health.borrow().voice_requested);
        assert!(health.borrow().error().is_none());
        sender.send_if_modified(|health| health.voice_state("current", false));
        sender.send_if_modified(|health| health.voice_state("current", true));
        events.update("current", true, false, None);
        assert!(!health.borrow().voice);
        assert!(!health.borrow().voice_requested);
        assert!(!health.borrow().voice_retry);
        assert!(health.borrow().error().is_some());
    }
}
