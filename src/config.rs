use anyhow::{Context, Result, bail, ensure};
use serenity::all::{ChannelId, GuildId};
use url::Url;

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
    for path in windows_config_paths(crate::store::app_data_dir(), std::env::current_exe().ok()) {
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

fn youtube_url(value: &str) -> Result<Url> {
    let value = value.trim();
    ensure!(!value.is_empty(), "Paste a YouTube URL first.");
    let url = Url::parse(value).context("Enter a complete YouTube URL (https://…).")?;
    ensure!(
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "Use a normal YouTube URL."
    );
    Ok(url)
}

pub(crate) fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && identifier(id)
}

fn identifier(id: &str) -> bool {
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Deliberately separate from video parsing: watch links import their playlist only here.
pub fn playlist_url(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.chars().any(char::is_control) && !value.contains('\\'),
        "Use a normal YouTube playlist URL."
    );
    // Parsing erases empty userinfo and standard ports; inspect authority before normalization.
    let authority = value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .filter(|authority| !authority.is_empty())
        .context("Enter a complete YouTube playlist URL (https://…).")?;
    ensure!(
        !authority.contains(['@', ':']),
        "Use a playlist URL without credentials or an explicit port."
    );
    let url = youtube_url(value)?;
    ensure!(
        matches!(
            url.host_str(),
            Some("youtube.com" | "www.youtube.com" | "music.youtube.com" | "m.youtube.com")
        ),
        "Use a youtube.com or music.youtube.com playlist URL."
    );
    ensure!(
        matches!(url.path(), "/playlist" | "/watch"),
        "Use a YouTube playlist or watch link with a list parameter."
    );
    if url.path() == "/watch" {
        video_url(value)?;
    }
    let mut ids = url.query_pairs().filter(|(key, _)| key == "list");
    let (_, id) = ids
        .next()
        .context("The URL needs a playlist identifier (list=…).")?;
    ensure!(
        ids.next().is_none() && (2..=128).contains(&id.len()) && identifier(&id),
        "Use one valid YouTube playlist identifier."
    );
    Ok(format!("https://www.youtube.com/playlist?list={id}"))
}

pub fn video_url(value: &str) -> Result<String> {
    let url = youtube_url(value)?;
    let id = match url.host_str() {
        Some("youtu.be") => url
            .path()
            .strip_prefix('/')
            .filter(|s| !s.contains('/'))
            .map(str::to_owned),
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com")
            if url.path() == "/watch" =>
        {
            let ids: Vec<_> = url.query_pairs().filter(|(key, _)| key == "v").collect();
            if ids.len() == 1 {
                Some(ids[0].1.to_string())
            } else {
                None
            }
        }
        Some("youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com") => None,
        _ => bail!("Only youtube.com, music.youtube.com and youtu.be videos are supported."),
    };
    let id = id
        .filter(|s| valid_video_id(s))
        .context("Use a direct video link, not a playlist or channel URL.")?;
    // Canonicalizing also discards playlist, tracking and start-time parameters.
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}

#[cfg(test)]
mod tests {
    use super::{parse_id, playlist_url, video_url};
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
    fn accepts_video_links_and_discards_playlist_parameters() -> Result<()> {
        let expected = "https://www.youtube.com/watch?v=BaW_jenozKc";
        for value in [
            " https://www.youtube.com/watch?v=BaW_jenozKc&list=PL123 ",
            "https://youtu.be/BaW_jenozKc?t=12",
            "https://music.youtube.com/watch?v=BaW_jenozKc",
            "http://m.youtube.com/watch?v=BaW_jenozKc",
        ] {
            assert_eq!(video_url(value)?, expected);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_or_unrelated_urls() {
        for value in [
            "",
            "not a url",
            "https://example.com/watch?v=BaW_jenozKc",
            "https://youtube.com.evil.com/watch?v=BaW_jenozKc",
            "https://youtube.com/playlist?list=123",
            "https://youtu.be/",
            "https://youtube.com/watch?v=short",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc",
            "https://user@youtube.com/watch?v=BaW_jenozKc",
            "https://youtube.com:444/watch?v=BaW_jenozKc",
            "file:///watch?v=BaW_jenozKc",
            "https://youtu.be/BaW_jenozKc/extra",
        ] {
            assert!(video_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn playlist_links_are_separate_from_single_video_links() -> Result<()> {
        let expected = "https://www.youtube.com/playlist?list=PL_test-123";
        for host in ["www.youtube.com", "youtube.com", "music.youtube.com"] {
            for path in [
                "playlist?list=PL_test-123",
                "watch?v=BaW_jenozKc&list=PL_test-123",
            ] {
                assert_eq!(playlist_url(&format!("https://{host}/{path}"))?, expected);
            }
        }
        assert!(video_url(expected).is_err());
        assert_eq!(
            video_url("https://youtube.com/watch?v=BaW_jenozKc&list=PL_test-123")?,
            "https://www.youtube.com/watch?v=BaW_jenozKc"
        );
        Ok(())
    }

    #[test]
    fn playlist_validation_rejects_ambiguous_and_untrusted_input() {
        for value in [
            "",
            "not a url",
            "https://youtube.com/playlist",
            "https://youtube.com/playlist?list=",
            "https://youtube.com/playlist?list=x",
            "https://youtube.com/playlist?list=PL%20bad",
            "https://youtube.com/playlist?list=PL%2Fbad",
            "https://youtube.com/playlist?list=PL$(id)",
            "https://youtube.com/playlist?list=PL123&list=PL123",
            "https://youtube.com/playlist?list=PL123&%6cist=PL456",
            "https://youtube.com/watch?list=PL123",
            "https://youtube.com/watch?v=x&list=PL123",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc&list=PL123",
            "https://example.com/playlist?list=PL123",
            "https://youtube.com.evil.org/playlist?list=PL123",
            "https://youtu.be/BaW_jenozKc?list=PL123",
            "https://user:pass@youtube.com/playlist?list=PL123",
            "https://@youtube.com/playlist?list=PL123",
            "https://youtube.com/playlist?li\nst=PL123",
            "https://youtube.com\\playlist?list=PL123",
            "https:youtube.com/playlist?list=PL123",
            "https:///youtube.com/playlist?list=PL123",
            "https://youtube.com:444/playlist?list=PL123",
            "file:///playlist?list=PL123",
            "https://youtube.com/channel?list=PL123",
            "https://youtube.com/playlist/extra?list=PL123",
        ] {
            assert!(playlist_url(value).is_err(), "accepted {value}");
        }
        assert!(
            playlist_url(&format!(
                "https://youtube.com/playlist?list={}",
                "P".repeat(129)
            ))
            .is_err()
        );
    }

    #[test]
    fn playlist_urls_reject_explicit_ports_before_url_normalization() {
        for authority in [
            "https://youtube.com:443",
            "http://youtube.com:80",
            "https://youtube.com:",
        ] {
            for path in ["/playlist?list=PL123", "/watch?v=BaW_jenozKc&list=PL123"] {
                assert!(
                    playlist_url(&format!("{authority}{path}")).is_err(),
                    "accepted {authority}{path}"
                );
            }
        }
        // Ordinary video parsing keeps its existing handling of standard ports.
        assert!(video_url("https://youtube.com:443/watch?v=BaW_jenozKc").is_ok());
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
