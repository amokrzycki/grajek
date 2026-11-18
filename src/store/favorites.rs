//! Durable favorites: a newest-first list of deliberately saved videos.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::safe_title;
use crate::config::video_url;

pub const FAVORITES_LIMIT: usize = 500;
const MAX_URL_BYTES: usize = 2_048;

/// A saved video. Order in [`super::Saved::favorites`] is save order, newest first;
/// the canonical URL is the identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteEntry {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    #[serde(default)]
    pub live: bool,
}

// One malformed entry must not discard siblings, preferences, queue or history.
pub(crate) fn read_favorites<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<FavoriteEntry>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// Canonicalizes, deduplicates, normalizes and bounds accepted favorites. Returns the
/// sanitized list plus the number of valid unique entries dropped beyond [`FAVORITES_LIMIT`].
pub(crate) fn sanitize(entries: Vec<FavoriteEntry>) -> (Vec<FavoriteEntry>, usize) {
    let mut clean: Vec<FavoriteEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for entry in entries {
        if entry.url.len() > MAX_URL_BYTES {
            continue;
        }
        let Ok(url) = video_url(&entry.url) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        clean.push(FavoriteEntry {
            url,
            title: entry.title.as_deref().and_then(safe_title),
            channel: entry.channel.as_deref().and_then(safe_title),
            duration_secs: entry.duration_secs,
            live: entry.live,
        });
    }
    let dropped = clean.len().saturating_sub(FAVORITES_LIMIT);
    clean.truncate(FAVORITES_LIMIT);
    (clean, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn favorite(id: &str, title: Option<&str>) -> FavoriteEntry {
        FavoriteEntry {
            url: format!("https://www.youtube.com/watch?v={id}"),
            title: title.map(str::to_owned),
            channel: None,
            duration_secs: None,
            live: false,
        }
    }

    #[test]
    fn canonicalizes_aliases_and_keeps_the_first_occurrence() {
        let entries = vec![
            FavoriteEntry {
                url: "https://youtu.be/aaaaaaaaaaa".into(),
                title: Some("Short link".into()),
                channel: Some("First channel".into()),
                duration_secs: Some(12),
                live: false,
            },
            FavoriteEntry {
                url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
                title: Some("Long link".into()),
                channel: Some("Second channel".into()),
                duration_secs: Some(99),
                live: true,
            },
            FavoriteEntry {
                url: "https://www.youtube.com/watch?v=bbbbbbbbbbb".into(),
                title: None,
                channel: None,
                duration_secs: None,
                live: false,
            },
        ];
        let (clean, dropped) = sanitize(entries);
        assert_eq!(dropped, 0);
        assert_eq!(clean.len(), 2);
        assert_eq!(clean[0].url, "https://www.youtube.com/watch?v=aaaaaaaaaaa");
        assert_eq!(clean[0].title.as_deref(), Some("Short link"));
        assert_eq!(clean[0].channel.as_deref(), Some("First channel"));
        assert_eq!(clean[0].duration_secs, Some(12));
        assert!(!clean[0].live);
        assert_eq!(clean[1].url, "https://www.youtube.com/watch?v=bbbbbbbbbbb");
        assert!(clean[1].title.is_none() && clean[1].channel.is_none());
    }

    #[test]
    fn drops_oversized_and_invalid_urls() {
        let entries = vec![
            favorite("aaaaaaaaaaa", Some("Valid")),
            FavoriteEntry {
                url: "x".repeat(2_049),
                ..favorite("bbbbbbbbbbb", None)
            },
            FavoriteEntry {
                url: "not a url".into(),
                ..favorite("ccccccccccc", None)
            },
            favorite("ddddddddddd", Some("Kept")),
        ];
        let (clean, dropped) = sanitize(entries);
        assert_eq!(dropped, 0);
        let urls: Vec<_> = clean.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.youtube.com/watch?v=aaaaaaaaaaa",
                "https://www.youtube.com/watch?v=ddddddddddd"
            ]
        );
    }

    #[test]
    fn normalizes_unicode_and_control_characters() {
        let entries = vec![FavoriteEntry {
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("  Hello\n\u{0}world\u{1}  ".into()),
            channel: Some("ż".repeat(300)),
            duration_secs: None,
            live: false,
        }];
        let (clean, _) = sanitize(entries);
        assert_eq!(clean[0].title.as_deref(), Some("Hello world"));
        assert_eq!(
            clean[0].channel.as_ref().map(|c| c.chars().count()),
            Some(200)
        );
    }

    #[test]
    fn missing_metadata_and_live_default_to_absent_or_false() -> anyhow::Result<()> {
        let saved: crate::store::Saved = serde_json::from_value(serde_json::json!({
            "favorites": [
                {"url": "https://www.youtube.com/watch?v=aaaaaaaaaaa"},
                {"url": "https://www.youtube.com/watch?v=bbbbbbbbbbb", "title": "Known"}
            ]
        }))?;
        assert_eq!(saved.favorites.len(), 2);
        assert!(saved.favorites[0].title.is_none());
        assert!(saved.favorites[0].channel.is_none());
        assert!(saved.favorites[0].duration_secs.is_none());
        assert!(!saved.favorites[0].live);
        assert_eq!(saved.favorites[1].title.as_deref(), Some("Known"));
        Ok(())
    }

    #[test]
    fn a_malformed_favorite_does_not_discard_siblings() -> anyhow::Result<()> {
        let saved: crate::store::Saved = serde_json::from_value(serde_json::json!({
            "volume": 42,
            "favorites": [
                null,
                {"title": "no url"},
                {"url": 42},
                {"url": "https://www.youtube.com/watch?v=aaaaaaaaaaa", "title": "Kept"}
            ]
        }))?;
        assert_eq!(saved.volume, 42);
        assert_eq!(saved.favorites.len(), 1);
        assert_eq!(saved.favorites[0].title.as_deref(), Some("Kept"));
        Ok(())
    }

    #[test]
    fn a_non_array_favorites_field_loads_empty() -> anyhow::Result<()> {
        let saved: crate::store::Saved =
            serde_json::from_str(r#"{"volume":42,"favorites":{"url":"x"}}"#)?;
        assert_eq!(saved.volume, 42);
        assert_eq!(saved.favorites.len(), 0);
        Ok(())
    }

    #[test]
    fn truncation_reports_only_valid_unique_overflow() {
        let entries: Vec<_> = (0..FAVORITES_LIMIT + 5)
            .map(|i| favorite(&format!("{i:0>11}"), Some("Song")))
            .collect();
        let (clean, dropped) = sanitize(entries);
        assert_eq!(clean.len(), FAVORITES_LIMIT);
        assert_eq!(dropped, 5);

        // Invalid entries beyond the limit do not count as overflow.
        let mut with_invalid: Vec<_> = (0..FAVORITES_LIMIT + 3)
            .map(|i| favorite(&format!("{i:0>11}"), None))
            .collect();
        with_invalid.push(FavoriteEntry {
            url: "https://evil.example/x".into(),
            ..favorite("zzzzzzzzzzz", None)
        });
        let (clean, dropped) = sanitize(with_invalid);
        assert_eq!(clean.len(), FAVORITES_LIMIT);
        assert_eq!(dropped, 3);
    }
}
