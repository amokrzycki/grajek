use serenity::all::GuildId;

use crate::discord::setup::{Server, Setup, Status};

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
    pub(super) fn guild_lost(&mut self, guild: GuildId, temporary: bool) {
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

    pub(super) fn reconcile_guilds(&mut self, guilds: &[serenity::all::UnavailableGuild]) {
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

    pub(super) fn guild_available(&mut self, server: Server) {
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

    pub(super) fn voice_state(&mut self, session: &str, in_channel: bool) -> bool {
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

pub(super) fn sort_servers(servers: &mut [Server]) {
    servers.sort_by_cached_key(|server| (server.name.to_lowercase(), server.id));
}
