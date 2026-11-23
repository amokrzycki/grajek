//! `YouTube` link parsing, canonical URLs and bounded display text.

use anyhow::{Context, Result, bail, ensure};
use url::Url;

/// Upper bound for any title or channel name shown or stored.
pub(crate) const MAX_TITLE_CHARS: usize = 200;

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

/// Hosts that serve canonical watch and playlist URLs.
pub(crate) fn is_watch_host(host: &str) -> bool {
    matches!(
        host,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com"
    )
}

/// Hosts that serve short `youtu.be` links.
pub(crate) fn is_short_host(host: &str) -> bool {
    matches!(host, "youtu.be" | "www.youtu.be")
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
        is_watch_host(url.host_str().unwrap_or_default()),
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
        Some(host) if is_watch_host(host) && url.path() == "/watch" => {
            let ids: Vec<_> = url.query_pairs().filter(|(key, _)| key == "v").collect();
            if ids.len() == 1 {
                Some(ids[0].1.to_string())
            } else {
                None
            }
        }
        Some(host) if is_watch_host(host) => None,
        _ => bail!("Only youtube.com, music.youtube.com and youtu.be videos are supported."),
    };
    let id = id
        .filter(|s| valid_video_id(s))
        .context("Use a direct video link, not a playlist or channel URL.")?;
    Ok(watch_url(&id))
}

/// Canonical watch URL for a validated video ID.
pub(crate) fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Collapses whitespace, drops control characters and bounds the result; empty text becomes `None`.
pub(crate) fn safe_title(title: &str) -> Option<String> {
    let title: String = title
        .chars()
        .filter(|ch| !ch.is_control() || ch.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    (!title.is_empty()).then_some(title)
}

/// Video ID from a canonical watch or `youtu.be` URL; other input is returned unchanged.
pub(crate) fn video_id(url: &str) -> &str {
    if let Some((_, id)) = url.rsplit_once("v=") {
        id.split('&').next().unwrap_or(id)
    } else if let Some((_, id)) = url.rsplit_once("youtu.be/") {
        id.split('?').next().unwrap_or(id)
    } else {
        url
    }
}

#[cfg(test)]
mod tests {
    use super::{is_short_host, is_watch_host, playlist_url, video_id, video_url};
    use anyhow::Result;

    #[test]
    fn host_allowlists_name_only_youtube_domains() {
        for host in [
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "music.youtube.com",
        ] {
            assert!(is_watch_host(host), "rejected watch host {host}");
        }
        for host in ["youtu.be", "www.youtu.be"] {
            assert!(is_short_host(host), "rejected short host {host}");
        }
        for host in ["", "example.com", "youtube.com.evil.com", "notyoutube.com"] {
            assert!(!is_watch_host(host), "accepted watch host {host}");
            assert!(!is_short_host(host), "accepted short host {host}");
        }
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
    fn extracts_video_ids_from_supported_urls() {
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=BaW_jenozKc"),
            "BaW_jenozKc"
        );
        assert_eq!(video_id("https://youtu.be/BaW_jenozKc"), "BaW_jenozKc");
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=BaW_jenozKc&t=123"),
            "BaW_jenozKc"
        );
        assert_eq!(
            video_id("https://youtu.be/BaW_jenozKc?t=123"),
            "BaW_jenozKc"
        );
    }
}
