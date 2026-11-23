use anyhow::{Result, ensure};
use serenity::all::{ChannelId, GuildId};

pub struct Config {
    pub warning: Option<String>,
    pub token: Option<String>,
    pub destination: Option<(GuildId, ChannelId)>,
}

impl Config {
    pub fn load() -> Self {
        let warning = match load_dotenv() {
            Ok(_) => None,
            Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => Some("Cannot read .env. Check its syntax and permissions. Existing environment settings still apply.".into()),
        };
        Self::from_values(
            std::env::var("DISCORD_BOT_TOKEN").ok(),
            std::env::var("DISCORD_GUILD_ID").ok(),
            std::env::var("DISCORD_VOICE_CHANNEL_ID").ok(),
            warning,
        )
    }

    fn from_values(
        token: Option<String>,
        guild: Option<String>,
        channel: Option<String>,
        warning: Option<String>,
    ) -> Self {
        let token = token.filter(|value| !value.trim().is_empty() && value != "your_bot_token");
        let destination = guild
            .zip(channel)
            .and_then(|(guild, channel)| parse_id(&guild).ok().zip(parse_id(&channel).ok()))
            .map(|(guild, channel)| (GuildId::new(guild), ChannelId::new(channel)));
        Self {
            warning,
            token,
            destination,
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn load_dotenv() -> dotenvy::Result<std::path::PathBuf> {
    dotenvy::dotenv()
}

/// Explorer launches use the per-user file first, then the executable's directory.
#[cfg(any(target_os = "windows", test))]
fn windows_config_paths(
    data: Option<std::path::PathBuf>,
    exe: Option<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    data.map(|path| path.join(".env"))
        .into_iter()
        .chain(exe.and_then(|path| path.parent().map(|parent| parent.join(".env"))))
        .collect()
}

#[cfg(target_os = "windows")]
fn load_dotenv() -> dotenvy::Result<std::path::PathBuf> {
    for path in windows_config_paths(crate::paths::app_data_dir(), std::env::current_exe().ok()) {
        match dotenvy::from_path(&path) {
            Ok(()) => return Ok(path),
            Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(dotenvy::Error::Io(std::io::ErrorKind::NotFound.into()))
}

fn parse_id(value: &str) -> Result<u64> {
    let id = value.trim().parse::<u64>()?;
    ensure!(id != 0, "ID cannot be zero");
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::parse_id;
    use anyhow::Result;

    #[test]
    fn dotenv_failure_preserves_external_credentials_and_invalid_ids_do_not_block_setup() {
        let config = super::Config::from_values(
            Some("external-secret".into()),
            Some("bad-id".into()),
            Some("2".into()),
            Some("Cannot read .env".into()),
        );
        assert_eq!(config.token.as_deref(), Some("external-secret"));
        assert!(config.destination.is_none());
        assert!(config.warning.is_some());
        let fresh = super::Config::from_values(None, None, None, None);
        assert!(fresh.token.is_none() && fresh.destination.is_none());
    }

    #[test]
    fn explorer_configuration_lookup_is_independent_of_working_directory() {
        let root = std::env::temp_dir();
        let data = root.join("Grajek");
        let executable = root.join("portable").join("grajek.exe");
        assert_eq!(
            super::windows_config_paths(Some(data.clone()), Some(executable.clone())),
            vec![data.join(".env"), root.join("portable/.env")]
        );
        assert_eq!(
            super::windows_config_paths(None, Some(executable)),
            vec![root.join("portable/.env")]
        );
        assert_eq!(
            super::windows_config_paths(None, None),
            Vec::<std::path::PathBuf>::new()
        );
    }

    #[test]
    fn ids_must_be_nonzero_numbers() -> Result<()> {
        assert_eq!(parse_id(" 123 ")?, 123);
        for value in ["0", "-1", "abc", "18446744073709551616", ""] {
            assert!(parse_id(value).is_err());
        }
        Ok(())
    }
}
