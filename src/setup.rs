//! Non-secret setup snapshots; credentials never implement Debug or Serialize.
use serenity::all::{ChannelId, GuildId, UserId};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Destination {
    pub bot: UserId,
    pub guild: GuildId,
    pub channel: ChannelId,
    pub guild_name: String,
    pub channel_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub id: GuildId,
    pub name: String,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    NeedsToken,
    Checking,
    NeedsDestination,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    pub status: Status,
    pub external: bool,
    pub remembered: bool,
    pub storage_notice: Option<String>,
    pub bot: Option<UserId>,
    pub servers: Vec<Server>,
    pub browsing: Option<GuildId>,
    pub channels: Vec<Channel>,
    pub destination: Option<Destination>,
}

pub enum Request {
    Restore {
        config: crate::config::Config,
        saved: Option<Destination>,
    },
    Authenticate {
        token: String,
        remember: bool,
    },
    Retry,
    Refresh,
    Channels(GuildId),
    Select(Destination),
    Remove,
}

impl Request {
    pub fn interrupts(&self) -> bool {
        matches!(
            self,
            Self::Retry
                | Self::Restore { .. }
                | Self::Authenticate { .. }
                | Self::Select(_)
                | Self::Remove
        )
    }
}

pub async fn credential(operation: Credential) -> Result<Option<String>, &'static str> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new("org.grajek.desktop", "discord-bot").map_err(
            |_| "Secure credential storage is unavailable. Choose session-only to continue.",
        )?;
        match operation {
            Credential::Read => match entry.get_password() {
                Ok(token) => Ok(Some(token)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => {
                    Err("Cannot unlock secure storage. Enter a token for this session or retry.")
                }
            },
            Credential::Save(token) => entry.set_password(&token).map(|()| None).map_err(
                |_| "Token was not saved. Unlock secure storage and retry, or choose session-only.",
            ),
            Credential::Remove => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err("Could not remove the saved token. Unlock secure storage and retry."),
            },
        }
    })
    .await
    .map_err(|_| "Secure storage could not be accessed.")?
}

pub enum Credential {
    Read,
    Save(String),
    Remove,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_does_not_interrupt_playback() {
        assert!(!Request::Refresh.interrupts());
        assert!(!Request::Channels(GuildId::new(1)).interrupts());
        assert!(Request::Remove.interrupts());
    }
    #[test]
    fn setup_snapshot_has_no_credentials() {
        let view = format!("{:?}", Setup::default());
        assert!(!view.contains("secret-token"));
        assert_eq!(Setup::default().status, Status::NeedsToken);
    }
}
