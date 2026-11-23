use anyhow::{Context as _, Result, ensure};
use serenity::all::{
    ChannelId, ChannelType, Client, GatewayIntents, GuildId, Http, Permissions, ShardManager,
    UserId,
};
use songbird::{CoreEvent, Event, SerenityInit, Songbird};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::timeout};

use super::gateway::GatewayEvents;
use super::health::Health;
use super::voice::VoiceEvents;
use crate::discord::setup::{Channel, Destination, Server, Status};

pub struct Discord {
    pub manager: Arc<Songbird>,
    pub guild: Option<GuildId>,
    pub(super) channel: Option<ChannelId>,
    pub(super) user: UserId,
    pub(super) http: Arc<Http>,
    pub health: watch::Receiver<Health>,
    pub(super) health_tx: watch::Sender<Health>,
    pub(super) shards: Arc<ShardManager>,
    pub(super) gateway: JoinHandle<()>,
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
            if client.start().await.is_err() {
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

pub(super) fn channel_reason(kind: ChannelType, permissions: Permissions) -> Option<String> {
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
