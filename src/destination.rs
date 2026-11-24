//! A selected Discord server and voice channel, persisted across restarts.

use serenity::all::{ChannelId, GuildId, UserId};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Destination {
    pub bot: UserId,
    pub guild: GuildId,
    pub channel: ChannelId,
    pub guild_name: String,
    pub channel_name: String,
}
