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
};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::timeout};

use crate::config::Config;

#[derive(Clone, Default)]
pub struct Health {
    pub gateway: bool,
    pub voice: bool,
    pub gateway_error: Option<String>,
    pub voice_error: Option<String>,
}

impl Health {
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
        if new.guild_id == Some(self.guild)
            && new.user_id == self.user
            && new.channel_id != Some(self.channel)
        {
            self.health.send_modify(|health| {
                health.voice = false;
                health.voice_error = Some("Bot left or was moved from the configured voice channel. Press Play to reconnect.".into());
            });
        }
    }
}

struct VoiceEvents(watch::Sender<Health>);

#[async_trait]
impl VoiceEventHandler for VoiceEvents {
    async fn act(&self, event: &EventContext<'_>) -> Option<Event> {
        match event {
            EventContext::DriverConnect(_) | EventContext::DriverReconnect(_) => {
                self.0.send_modify(|health| {
                    health.voice = true;
                    health.voice_error = None;
                });
            }
            EventContext::DriverDisconnect(data) => {
                // Never log the voice session ID or the entire event context.
                tracing::warn!(kind = ?data.kind, reason = ?data.reason, "Discord voice disconnected");
                self.0.send_modify(|health| {
                    health.voice = false;
                    health.voice_error =
                        Some("Discord voice disconnected. Press Play to reconnect.".into());
                });
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
        self.health_tx
            .send_modify(|health| health.voice_error = None);
    }

    pub async fn connect(&self) -> Result<()> {
        let call = self.manager.get_or_insert(self.guild);
        let already_joined =
            call.lock().await.current_channel().map(|id| id.0.get()) == Some(self.channel.get());
        if already_joined && self.health.borrow().voice {
            return Ok(());
        }
        self.check_channel().await?;
        {
            let mut call = call.lock().await;
            call.remove_all_global_events();
            for event in [
                CoreEvent::DriverConnect,
                CoreEvent::DriverReconnect,
                CoreEvent::DriverDisconnect,
            ] {
                call.add_global_event(Event::Core(event), VoiceEvents(self.health_tx.clone()));
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
        self.health_tx.send_modify(|health| {
            health.voice = true;
            health.voice_error = None;
        });
        tracing::info!("Connected to Discord voice with Songbird's DAVE-capable driver");
        Ok(())
    }

    pub async fn reset_incomplete_connection(&self) {
        if !self.health.borrow().voice {
            let _ = timeout(Duration::from_secs(3), self.manager.remove(self.guild)).await;
        }
    }

    pub async fn shutdown(&self) {
        if let Some(call) = self.manager.get(self.guild) {
            call.lock().await.stop();
        }
        let _ = timeout(Duration::from_secs(3), self.manager.remove(self.guild)).await;
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
