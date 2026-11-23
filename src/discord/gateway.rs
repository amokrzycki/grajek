use serenity::{
    all::{
        ConnectionStage, Context, EventHandler, Ready, ShardStageUpdateEvent, UserId, VoiceState,
    },
    async_trait,
};
use tokio::sync::watch;

use super::health::{Health, sort_servers};
use crate::discord::setup::{Server, Status};

pub(super) struct GatewayEvents {
    pub(super) epoch: u64,
    pub(super) ready: watch::Sender<bool>,
    pub(super) health: watch::Sender<Health>,
    pub(super) user: UserId,
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
