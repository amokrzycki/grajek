use anyhow::{Context, Result, bail, ensure};
use serenity::all::{ChannelId, GuildId};
use url::Url;

pub struct Config {
    pub token: String,
    pub guild: GuildId,
    pub channel: ChannelId,
}

impl Config {
    pub fn load() -> Result<Self> {
        match dotenvy::dotenv() {
            Ok(_) => {}
            Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Cannot read .env. Check its syntax and permissions."),
        }
        let token = std::env::var("DISCORD_BOT_TOKEN")
            .context("Set DISCORD_BOT_TOKEN in .env, then restart.")?;
        ensure!(
            !token.trim().is_empty() && token != "your_bot_token",
            "Set a real DISCORD_BOT_TOKEN in .env, then restart."
        );
        Ok(Self {
            token,
            guild: GuildId::new(env_id("DISCORD_GUILD_ID")?),
            channel: ChannelId::new(env_id("DISCORD_VOICE_CHANNEL_ID")?),
        })
    }
}

fn env_id(name: &str) -> Result<u64> {
    let value =
        std::env::var(name).with_context(|| format!("Set {name} in .env, then restart."))?;
    parse_id(&value).with_context(|| format!("{name} must be a nonzero numeric Discord ID."))
}

fn parse_id(value: &str) -> Result<u64> {
    let id = value.trim().parse::<u64>()?;
    ensure!(id != 0, "ID cannot be zero");
    Ok(id)
}

pub fn video_url(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "Paste a YouTube video URL first.");
    let url = Url::parse(value).context("Enter a complete YouTube URL (https://…).")?;
    ensure!(
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "Use a normal YouTube video URL."
    );
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
        .filter(|s| {
            s.len() == 11
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .context("Use a direct video link, not a playlist or channel URL.")?;
    // Canonicalizing also discards playlist, tracking and start-time parameters.
    Ok(format!("https://www.youtube.com/watch?v={id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_video_links_and_discards_playlist_parameters() {
        let expected = "https://www.youtube.com/watch?v=BaW_jenozKc";
        for value in [
            " https://www.youtube.com/watch?v=BaW_jenozKc&list=PL123 ",
            "https://youtu.be/BaW_jenozKc?t=12",
            "https://music.youtube.com/watch?v=BaW_jenozKc",
            "http://m.youtube.com/watch?v=BaW_jenozKc",
        ] {
            assert_eq!(video_url(value).unwrap(), expected);
        }
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
    fn ids_must_be_nonzero_numbers() {
        assert_eq!(parse_id(" 123 ").unwrap(), 123);
        for value in ["0", "-1", "abc", "18446744073709551616", ""] {
            assert!(parse_id(value).is_err());
        }
    }
}
