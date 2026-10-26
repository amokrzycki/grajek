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

use crate::setup::{Channel, Destination, Server, Setup, Status};

#[derive(Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "gateway, voice, join intent and retry eligibility are independent facts"
)]
pub struct Health {
    pub discovery_epoch: u64,
    pub session_epoch: u64,
    pub setup: Setup,
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
    fn guild_lost(&mut self, guild: GuildId, temporary: bool) {
        self.discovery_epoch = self.discovery_epoch.wrapping_add(1);
        if temporary {
            if let Some(server) = self
                .setup
                .servers
                .iter_mut()
                .find(|server| server.id == guild)
            {
                server.available = false;
            }
        } else {
            self.setup.servers.retain(|server| server.id != guild);
        }
        if self
            .setup
            .destination
            .as_ref()
            .is_some_and(|destination| destination.guild == guild)
        {
            if temporary {
                self.setup.status = Status::Failed(
                    "Selected server is temporarily unavailable. Wait for Discord or refresh."
                        .into(),
                );
            } else {
                self.setup.destination = None;
                self.setup.status = Status::NeedsDestination;
            }
            self.voice_error = Some(
                "Selected server is unavailable. Playback stopped; retry when it is available."
                    .into(),
            );
            self.voice_requested = false;
            self.voice_retry = false;
            self.voice_epoch = self.voice_epoch.wrapping_add(1);
        }
        if self.setup.browsing == Some(guild) {
            self.setup.channels.clear();
        }
    }

    pub(crate) fn refresh_servers(&mut self, epoch: u64, mut servers: Vec<Server>) -> bool {
        if self.discovery_epoch != epoch {
            return false;
        }
        let lost = self
            .setup
            .destination
            .as_ref()
            .map(|destination| destination.guild)
            .filter(|guild| !servers.iter().any(|server| server.id == *guild));
        if let Some(guild) = lost {
            self.guild_lost(guild, false);
        }
        sort_servers(&mut servers);
        self.setup.servers = servers;
        lost.is_some()
    }

    fn reconcile_guilds(&mut self, guilds: &[serenity::all::UnavailableGuild]) {
        self.discovery_epoch = self.discovery_epoch.wrapping_add(1);
        if let Some(guild) = self
            .setup
            .destination
            .as_ref()
            .map(|destination| destination.guild)
        {
            match guilds.iter().find(|entry| entry.id == guild) {
                None => self.guild_lost(guild, false),
                Some(entry) if entry.unavailable => self.guild_lost(guild, true),
                Some(_) => {}
            }
        }
        self.setup
            .servers
            .retain(|server| guilds.iter().any(|guild| guild.id == server.id));
        for server in &mut self.setup.servers {
            server.available = guilds
                .iter()
                .any(|guild| guild.id == server.id && !guild.unavailable);
        }
    }

    fn guild_available(&mut self, server: Server) {
        self.discovery_epoch = self.discovery_epoch.wrapping_add(1);
        let id = server.id;
        self.setup.servers.retain(|entry| entry.id != id);
        self.setup.servers.push(server);
        if self
            .setup
            .destination
            .as_ref()
            .is_some_and(|destination| destination.guild == id)
        {
            self.setup.status = Status::Ready;
        }
        sort_servers(&mut self.setup.servers);
    }

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
    epoch: u64,
    ready: watch::Sender<bool>,
    health: watch::Sender<Health>,
    user: UserId,
}

#[async_trait]
impl EventHandler for GatewayEvents {
    async fn ready(&self, _: Context, ready: Ready) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.reconcile_guilds(&ready.guilds);
            health.gateway = true;
            health.gateway_error = None;
        });
        self.ready.send_replace(true);
        tracing::info!("Discord gateway ready");
    }

    async fn guild_create(&self, _: Context, guild: serenity::all::Guild, _: Option<bool>) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.guild_available(Server {
                id: guild.id,
                name: guild.name,
                available: true,
            });
        });
    }

    async fn guild_update(
        &self,
        _: Context,
        _: Option<serenity::all::Guild>,
        guild: serenity::all::PartialGuild,
    ) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.discovery_epoch = health.discovery_epoch.wrapping_add(1);
            if let Some(server) = health
                .setup
                .servers
                .iter_mut()
                .find(|server| server.id == guild.id)
            {
                server.name.clone_from(&guild.name);
            }
            if let Some(destination) = health
                .setup
                .destination
                .as_mut()
                .filter(|destination| destination.guild == guild.id)
            {
                destination.guild_name = guild.name;
            }
            sort_servers(&mut health.setup.servers);
        });
    }

    async fn channel_update(
        &self,
        _: Context,
        _: Option<serenity::all::GuildChannel>,
        channel: serenity::all::GuildChannel,
    ) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.discovery_epoch = health.discovery_epoch.wrapping_add(1);
            if health.setup.browsing == Some(channel.guild_id) {
                health.setup.channels.clear();
            }
            if let Some(destination) = health
                .setup
                .destination
                .as_mut()
                .filter(|destination| destination.channel == channel.id)
            {
                destination.channel_name = channel.name;
            }
        });
    }

    async fn guild_delete(
        &self,
        _: Context,
        guild: serenity::all::UnavailableGuild,
        _: Option<serenity::all::Guild>,
    ) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.guild_lost(guild.id, guild.unavailable);
        });
    }

    async fn channel_delete(
        &self,
        _: Context,
        channel: serenity::all::GuildChannel,
        _: Option<Vec<serenity::all::Message>>,
    ) {
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.discovery_epoch = health.discovery_epoch.wrapping_add(1);
            health.setup.channels.retain(|entry| entry.id != channel.id);
            if health
                .setup
                .destination
                .as_ref()
                .is_some_and(|destination| destination.channel == channel.id)
            {
                health.setup.destination = None;
                health.setup.status = Status::NeedsDestination;
                health.voice_error =
                    Some("Selected voice channel was removed. Choose another in Settings.".into());
                health.voice_requested = false;
                health.voice_retry = false;
                health.voice_epoch = health.voice_epoch.wrapping_add(1);
            }
        });
    }

    async fn shard_stage_update(&self, _: Context, event: ShardStageUpdateEvent) {
        let connected = event.new == ConnectionStage::Connected;
        self.health.send_modify(|health| {
            if health.session_epoch != self.epoch {
                return;
            }
            health.gateway = connected;
            health.gateway_error = if connected {
                None
            } else {
                Some("Discord gateway reconnecting. Check the network.".into())
            };
        });
    }

    async fn voice_state_update(&self, _: Context, _: Option<VoiceState>, new: VoiceState) {
        if new.user_id == self.user {
            self.health.send_if_modified(|health| {
                if health.session_epoch != self.epoch {
                    return false;
                }
                let Some(destination) = &health.setup.destination else {
                    return false;
                };
                if new.guild_id != Some(destination.guild) {
                    return false;
                }
                health.voice_state(&new.session_id, new.channel_id == Some(destination.channel))
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
    pub guild: Option<GuildId>,
    channel: Option<ChannelId>,
    user: UserId,
    http: Arc<Http>,
    pub health: watch::Receiver<Health>,
    health_tx: watch::Sender<Health>,
    shards: Arc<ShardManager>,
    gateway: JoinHandle<()>,
}

impl Discord {
    pub async fn start(token: &str, health_tx: watch::Sender<Health>) -> Result<Self> {
        let http = Arc::new(Http::new(token));
        let user = timeout(Duration::from_secs(15), http.get_current_user())
            .await
            .context("Discord authentication timed out. Check the network.")?
            .map_err(auth_error)?;
        ensure!(user.bot, "Use a Discord bot token, never a user token.");
        let health = health_tx.subscribe();
        health_tx.send_modify(|health| {
            health.setup.bot = Some(user.id);
            health.setup.status = Status::NeedsDestination;
        });
        let (ready_tx, mut ready) = watch::channel(false);
        let manager = Songbird::serenity();
        let epoch = health_tx.borrow().session_epoch;
        let mut client = timeout(Duration::from_secs(15), async {
            Client::builder(
                token,
                GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES,
            )
            .event_handler(GatewayEvents {
                epoch,
                ready: ready_tx,
                health: health_tx.clone(),
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
        let gateway_epoch = health_tx.borrow().session_epoch;
        let gateway = tokio::spawn(async move {
            if let Err(error) = client.start().await {
                let _ = error;
                tracing::error!("Discord gateway failed");
                gateway_health.send_modify(|health| {
                    if health.session_epoch != gateway_epoch {
                        return;
                    }
                    health.gateway = false;
                    health.gateway_error = Some(
                        "Discord gateway failed. Check the token/network, retry in Settings."
                            .into(),
                    );
                });
            }
        });
        let discord = Self {
            manager,
            guild: None,
            channel: None,
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
            Ok(())
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

    pub async fn guilds(&self, after: Option<GuildId>) -> Result<Vec<Server>> {
        let page = self
            .http
            .get_guilds(after.map(serenity::http::GuildPagination::After), Some(200))
            .await
            .context("Could not refresh servers. Check the network and retry.")?;
        Ok(page
            .into_iter()
            .map(|guild| Server {
                id: guild.id,
                name: guild.name,
                available: true,
            })
            .collect())
    }

    pub async fn channels(&self, guild_id: GuildId) -> Result<Vec<Channel>> {
        let guild = guild_id
            .to_partial_guild(&self.http)
            .await
            .context("Server unavailable. Refresh and check that the bot is still invited.")?;
        let member = guild_id
            .member(&self.http, self.user)
            .await
            .context("Bot cannot access this server. Invite it and refresh.")?;
        let channels = guild_id
            .channels(&self.http)
            .await
            .context("Could not load voice channels. Check the network and retry.")?;
        let mut result: Vec<_> = channels
            .into_values()
            .filter(|channel| matches!(channel.kind, ChannelType::Voice | ChannelType::Stage))
            .map(|channel| {
                let permissions = guild.user_permissions_in(&channel, &member);
                let reason = channel_reason(channel.kind, permissions);
                Channel {
                    id: channel.id,
                    name: channel.name,
                    reason,
                }
            })
            .collect();
        result.sort_by_cached_key(|channel| (channel.name.to_lowercase(), channel.id));
        Ok(result)
    }

    pub async fn select(&mut self, mut destination: Destination) -> Result<()> {
        ensure!(
            destination.bot == self.user,
            "Bot changed. Select a destination again."
        );
        let server = self
            .health
            .borrow()
            .setup
            .servers
            .iter()
            .find(|server| server.id == destination.guild && server.available)
            .cloned()
            .context("Server unavailable. Refresh the server list.")?;
        let channels = self.channels(destination.guild).await?;
        let channel = channels
            .into_iter()
            .find(|channel| channel.id == destination.channel)
            .context("Voice channel no longer exists. Refresh the channel list.")?;
        ensure!(
            channel.reason.is_none(),
            "Bot needs View Channel, Connect and Speak in an ordinary voice channel."
        );
        self.disconnect().await?;
        ensure!(
            self.health
                .borrow()
                .setup
                .servers
                .iter()
                .any(|server| server.id == destination.guild && server.available),
            "Server changed during selection. Refresh and retry."
        );
        destination.guild_name = server.name;
        destination.channel_name = channel.name;
        self.guild = Some(destination.guild);
        self.channel = Some(destination.channel);
        self.health_tx.send_modify(|health| {
            health.setup.destination = Some(destination);
            health.setup.status = Status::Ready;
        });
        Ok(())
    }

    async fn check_channel(&self) -> Result<()> {
        let guild = self.guild.context("Choose a server in Settings.")?;
        let channel = self
            .channel
            .context("Choose a voice channel in Settings.")?;
        let channels = timeout(Duration::from_secs(15), self.channels(guild))
            .await
            .context("Checking voice permissions timed out. Retry.")??;
        ensure!(
            channels
                .iter()
                .any(|entry| entry.id == channel && entry.reason.is_none()),
            "Voice channel unavailable. Check View Channel, Connect and Speak, then refresh Settings."
        );
        Ok(())
    }

    pub async fn connect(&self) -> Result<()> {
        let guild = self.guild.context("Choose a server in Settings.")?;
        let channel = self
            .channel
            .context("Choose a voice channel in Settings.")?;
        let call = self.manager.get_or_insert(guild);
        let already_joined =
            call.lock().await.current_channel().map(|id| id.0.get()) == Some(channel.get());
        // Cancelled teardown may leave voice connected with its callbacks invalidated.
        let voice_active = {
            let health = self.health.borrow();
            health.voice && health.voice_requested
        };
        if already_joined && voice_active {
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
        let call = self.manager.get_or_insert(guild);
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
        timeout(Duration::from_secs(20), self.manager.join(guild, channel))
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
        // Invalidate callbacks before teardown, but keep the last confirmed voice state.
        self.health_tx.send_modify(|health| {
            health.voice_epoch = health.voice_epoch.wrapping_add(1);
            health.voice_requested = false;
            health.voice_session = None;
            health.voice_retry = false;
            health.voice_error = None;
        });
        let Some(guild) = self.guild else {
            return Ok(());
        };
        let result = if self.manager.get(guild).is_none() {
            Ok(())
        } else {
            timeout(Duration::from_secs(3), self.manager.remove(guild))
                .await
                .context("Leaving Discord voice timed out. Try Leave voice again.")
                .and_then(|result| {
                    result.context("Could not leave Discord voice. Check the network.")
                })
        };
        self.health_tx.send_modify(|health| {
            if result.is_ok() {
                health.voice = false;
            }
            health.voice_error = result.as_ref().err().map(ToString::to_string);
        });
        result
    }

    pub async fn reset_incomplete_connection(&self) {
        if !self.health.borrow().voice {
            let _ = self.disconnect().await;
        }
    }

    pub async fn shutdown(&self) {
        if let Some(call) = self.guild.and_then(|guild| self.manager.get(guild)) {
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
        let shards = self.shards.clone();
        tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(3), shards.shutdown_all()).await;
        });
    }
}

fn channel_reason(kind: ChannelType, permissions: Permissions) -> Option<String> {
    if kind != ChannelType::Voice {
        Some("Stage channels are not supported".into())
    } else if !permissions
        .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK)
    {
        Some("Needs View Channel, Connect and Speak".into())
    } else {
        None
    }
}

fn sort_servers(servers: &mut [Server]) {
    servers.sort_by_cached_key(|server| (server.name.to_lowercase(), server.id));
}

fn auth_error(error: serenity::Error) -> anyhow::Error {
    let message = match error {
        serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response))
            if response.status_code.as_u16() == 401 =>
        {
            "Invalid bot token. Copy the bot token from the Developer Portal and retry."
        }
        serenity::Error::Http(serenity::http::HttpError::Request(_)) => {
            "Network unavailable. Check your connection and retry."
        }
        _ => "Discord authentication failed temporarily. Retry shortly.",
    };
    anyhow::anyhow!(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_require_all_permissions_and_never_allow_stage() {
        let required = Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK;
        assert!(channel_reason(ChannelType::Voice, required).is_none());
        for missing in [
            Permissions::VIEW_CHANNEL,
            Permissions::CONNECT,
            Permissions::SPEAK,
        ] {
            assert!(channel_reason(ChannelType::Voice, required - missing).is_some());
        }
        assert!(channel_reason(ChannelType::Stage, Permissions::all()).is_some());
    }

    #[test]
    fn guild_sort_is_deterministic_with_unicode_and_duplicate_names() {
        let mut servers = vec![
            Server {
                id: GuildId::new(3),
                name: "żółć".into(),
                available: false,
            },
            Server {
                id: GuildId::new(2),
                name: "Alpha".into(),
                available: true,
            },
            Server {
                id: GuildId::new(1),
                name: "alpha".into(),
                available: true,
            },
        ];
        sort_servers(&mut servers);
        assert_eq!(
            servers
                .iter()
                .map(|server| server.id.get())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    fn selected_health() -> Health {
        Health {
            setup: Setup {
                destination: Some(Destination {
                    bot: UserId::new(3),
                    guild: GuildId::new(1),
                    channel: ChannelId::new(2),
                    guild_name: "Server".into(),
                    channel_name: "Voice".into(),
                }),
                servers: vec![Server {
                    id: GuildId::new(1),
                    name: "Server".into(),
                    available: true,
                }],
                ..Setup::default()
            },
            voice_requested: true,
            voice_epoch: 4,
            ..Health::default()
        }
    }

    #[test]
    fn temporary_guild_outage_keeps_selection_and_return_does_not_rejoin() {
        let mut health = selected_health();
        let selected = health.setup.destination.clone();
        health.guild_lost(GuildId::new(1), true);
        assert_eq!(health.setup.destination, selected);
        assert!(!health.setup.servers[0].available && !health.voice_requested);
        assert_eq!(health.voice_epoch, 5);
        health.guild_available(Server {
            id: GuildId::new(1),
            name: "Server".into(),
            available: true,
        });
        assert_eq!(health.setup.destination, selected);
        assert!(health.setup.servers[0].available);
        assert!(!health.voice && !health.voice_requested && !health.voice_retry);
    }

    #[test]
    fn ready_reconciles_missed_removal_and_invalidates_discovery_results() {
        let mut health = selected_health();
        let before = health.discovery_epoch;
        health.reconcile_guilds(&[]);
        assert!(health.setup.destination.is_none());
        assert!(health.discovery_epoch > before);
        assert!(!health.voice_requested && !health.voice_retry);
        assert_eq!(health.setup.servers.len(), 0);
    }

    #[test]
    fn rest_refresh_removal_requests_disconnect_and_invalidates_selection() {
        let mut health = selected_health();
        health.setup.status = Status::Ready;
        health.voice = true;
        health.voice_retry = true;
        let epoch = health.discovery_epoch;
        assert!(health.refresh_servers(epoch, Vec::new()));
        assert!(health.setup.destination.is_none());
        assert!(matches!(health.setup.status, Status::NeedsDestination));
        assert!(!health.voice_requested && !health.voice_retry);
        assert_eq!(health.voice_epoch, 5);
        // Keep confirmed voice state until disconnect succeeds so failure remains retryable.
        assert!(health.voice);
        assert!(!health.refresh_servers(epoch, Vec::new()));
    }

    #[test]
    fn stale_rest_refresh_cannot_remove_current_destination() {
        let mut health = selected_health();
        health.discovery_epoch = 3;
        let selected = health.setup.destination.clone();
        assert!(!health.refresh_servers(2, Vec::new()));
        assert_eq!(health.setup.destination, selected);
        assert!(health.voice_requested);
    }

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

    #[tokio::test]
    async fn cancelled_idle_disconnect_requires_a_fresh_connection() -> Result<()> {
        use std::{future::Future as _, task::Poll};

        let manager = Songbird::serenity();
        manager.initialise_client_data(1, UserId::new(3));
        let guild = GuildId::new(1);
        let channel = ChannelId::new(2);
        let call = manager.get_or_insert(guild);
        // Songbird buffers this gateway request without opening a network connection.
        drop(call.lock().await.join_gateway(channel).await?);
        let (events, health) = events();
        // Permission checks fail locally, so only a valid existing connection can succeed.
        let http = Arc::new(
            serenity::http::HttpBuilder::new("unused-test-token")
                .proxy("invalid proxy")
                .build(),
        );
        let (shards, _) = ShardManager::new(serenity::gateway::ShardManagerOptions {
            data: Arc::new(tokio::sync::RwLock::new(serenity::prelude::TypeMap::new())),
            event_handlers: vec![],
            raw_event_handlers: vec![],
            shard_index: 0,
            shard_init: 0,
            shard_total: 1,
            voice_manager: None,
            ws_url: Arc::new(tokio::sync::Mutex::new(String::new())),
            http: http.clone(),
            intents: GatewayIntents::empty(),
            presence: None,
        });
        let discord = Discord {
            manager,
            guild: Some(guild),
            channel: Some(channel),
            user: UserId::new(3),
            http,
            health,
            health_tx: events.health,
            shards,
            gateway: tokio::spawn(std::future::pending()),
        };
        assert_eq!(
            call.lock().await.current_channel().map(|id| id.0.get()),
            Some(channel.get())
        );
        discord.connect().await?;

        let guard = call.lock().await;
        let mut leave = Box::pin(discord.disconnect());
        assert!(std::future::poll_fn(|cx| Poll::Ready(leave.as_mut().poll(cx).is_pending())).await);
        // Play cancels idle teardown while Songbird's call lock is held.
        drop(leave);
        drop(guard);
        assert!(discord.health.borrow().voice);
        assert!(!discord.health.borrow().voice_requested);
        assert!(discord.connect().await.is_err());
        Ok(())
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
