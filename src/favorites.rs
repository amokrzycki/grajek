//! Durable favorites: a newest-first list of deliberately saved videos.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::youtube::{safe_title, video_url};

pub const FAVORITES_LIMIT: usize = 500;
const MAX_URL_BYTES: usize = 2_048;

pub(crate) fn is_full(len: usize) -> bool {
    len >= FAVORITES_LIMIT
}

pub(crate) fn full_message() -> String {
    format!("Favorites full ({FAVORITES_LIMIT} tracks). Remove a favorite and try again.")
}

/// A saved video. Order is save order, newest first; the canonical URL is the identity.
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

impl FavoriteEntry {
    /// Canonicalizes the URL and normalizes text, dropping what cannot be saved.
    fn canonicalized(self) -> Option<Self> {
        if self.url.len() > MAX_URL_BYTES {
            return None;
        }
        Some(Self {
            url: video_url(&self.url).ok()?,
            title: self.title.as_deref().and_then(safe_title),
            channel: self.channel.as_deref().and_then(safe_title),
            duration_secs: self.duration_secs,
            live: self.live,
        })
    }

    /// Fills facts this entry lacks; known values are never replaced.
    fn merge_facts(&mut self, newer: Self) {
        if self.title.is_none() {
            self.title = newer.title;
        }
        if self.channel.is_none() {
            self.channel = newer.channel;
        }
        if self.duration_secs.is_none() {
            self.duration_secs = newer.duration_secs;
        }
        self.live |= newer.live;
    }
}

/// The position of the entry with this canonical URL; the one identity rule.
// ponytail: linear scan over at most FAVORITES_LIMIT entries; index it if the limit grows.
fn position(entries: &[FavoriteEntry], canonical_url: &str) -> Option<usize> {
    entries.iter().position(|known| known.url == canonical_url)
}

pub(crate) fn contains(entries: &[FavoriteEntry], canonical_url: &str) -> bool {
    position(entries, canonical_url).is_some()
}

/// Applies a desired membership state to a canonical entry. An accept either inserts it at
/// the front, merges still-missing facts into the known entry, or removes it.
pub(crate) fn apply(
    entries: &mut Vec<FavoriteEntry>,
    entry: FavoriteEntry,
    saved: bool,
) -> Result<(), FavoriteRefusal> {
    let Some(entry) = entry.canonicalized() else {
        return Err(FavoriteRefusal::InvalidUrl);
    };
    match position(entries, &entry.url) {
        Some(index) if saved => entries[index].merge_facts(entry),
        Some(index) => {
            entries.remove(index);
        }
        None if !saved => {}
        None if is_full(entries.len()) => return Err(FavoriteRefusal::Full),
        None => entries.insert(0, entry),
    }
    Ok(())
}

/// Canonicalizes, deduplicates, normalizes and bounds accepted favorites. Returns the
/// sanitized list plus the number of valid unique entries dropped beyond [`FAVORITES_LIMIT`].
pub(crate) fn sanitize(entries: Vec<FavoriteEntry>) -> (Vec<FavoriteEntry>, usize) {
    let mut clean: Vec<FavoriteEntry> = Vec::new();
    for entry in entries {
        let Some(entry) = entry.canonicalized() else {
            continue;
        };
        if position(&clean, &entry.url).is_none() {
            clean.push(entry);
        }
    }
    let dropped = clean.len().saturating_sub(FAVORITES_LIMIT);
    clean.truncate(FAVORITES_LIMIT);
    (clean, dropped)
}

/// Why a favorites membership request was refused. Independent of playback phase and errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FavoriteRefusal {
    InvalidUrl,
    StaleSearch,
    Full,
}

impl fmt::Display for FavoriteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => f.write_str("Invalid link."),
            Self::StaleSearch => {
                f.write_str("That search result is no longer available. Search again.")
            }
            Self::Full => f.write_str(&full_message()),
        }
    }
}

/// Answer to a favorites membership request. `result` is `Ok` when the desired state was
/// applied; `Err` carries a refusal without changing membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FavoriteResult {
    pub request_id: u64,
    pub url: String,
    pub saved: bool,
    pub result: Result<(), FavoriteRefusal>,
}

#[cfg(test)]
mod tests {
    use super::{FAVORITES_LIMIT, FavoriteEntry, FavoriteRefusal, apply, contains, sanitize};

    fn favorite(id: &str, title: Option<&str>) -> FavoriteEntry {
        crate::test_support::favorite(&format!("https://www.youtube.com/watch?v={id}"), title)
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

    #[test]
    fn saving_an_alias_merges_missing_facts_into_the_known_entry() {
        let mut entries = vec![FavoriteEntry {
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("Known".into()),
            channel: None,
            duration_secs: None,
            live: false,
        }];
        let newer = FavoriteEntry {
            url: "https://youtu.be/aaaaaaaaaaa".into(),
            title: Some("Newer".into()),
            channel: Some("A channel".into()),
            duration_secs: Some(42),
            live: true,
        };
        assert_eq!(apply(&mut entries, newer, true), Ok(()));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_deref(), Some("Known"));
        assert_eq!(entries[0].channel.as_deref(), Some("A channel"));
        assert_eq!(entries[0].duration_secs, Some(42));
        assert!(entries[0].live);
    }

    #[test]
    fn absent_urls_insert_at_the_front_and_respect_the_limit() {
        let mut entries = Vec::new();
        for entry in [
            favorite("aaaaaaaaaaa", Some("A")),
            favorite("bbbbbbbbbbb", Some("B")),
        ] {
            assert_eq!(apply(&mut entries, entry, true), Ok(()));
        }
        assert_eq!(
            entries[0].url,
            "https://www.youtube.com/watch?v=bbbbbbbbbbb"
        );
        assert!(contains(
            &entries,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa"
        ));

        let remove = favorite("bbbbbbbbbbb", None);
        assert_eq!(apply(&mut entries, remove.clone(), false), Ok(()));
        assert_eq!(apply(&mut entries, remove, false), Ok(()));
        assert_eq!(entries.len(), 1);

        let mut full: Vec<_> = (0..FAVORITES_LIMIT)
            .map(|i| favorite(&format!("{i:0>11}"), None))
            .collect();
        assert_eq!(
            apply(&mut full, favorite("zzzzzzzzzzz", None), true),
            Err(FavoriteRefusal::Full)
        );
        assert_eq!(full.len(), FAVORITES_LIMIT);
        assert_eq!(
            apply(&mut full, favorite("00000000000", None), false),
            Ok(())
        );
        assert_eq!(full.len(), FAVORITES_LIMIT - 1);
    }

    #[test]
    fn invalid_urls_are_refused_without_touching_membership() {
        let mut entries = vec![favorite("aaaaaaaaaaa", None)];
        let invalid = FavoriteEntry {
            url: "not a url".into(),
            ..favorite("bbbbbbbbbbb", None)
        };
        assert_eq!(
            apply(&mut entries, invalid, true),
            Err(FavoriteRefusal::InvalidUrl)
        );
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn refusals_explain_themselves() {
        assert!(
            FavoriteRefusal::Full
                .to_string()
                .contains(&FAVORITES_LIMIT.to_string())
        );
        assert_eq!(FavoriteRefusal::InvalidUrl.to_string(), "Invalid link.");
        assert!(
            FavoriteRefusal::StaleSearch
                .to_string()
                .contains("Search again")
        );
    }
}
