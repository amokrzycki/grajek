//! Durable playback history: videos that actually started playing.

use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const HISTORY_LIMIT: usize = 50;

/// A video that actually started playing. Only durable facts are stored: the canonical
/// watch URL, never the expiring media-stream URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

impl HistoryEntry {
    pub fn duration(&self) -> Option<Duration> {
        self.duration_secs.map(Duration::from_secs)
    }
}

/// Moves a replayed video to the top instead of duplicating it.
pub fn record(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    entries.retain(|known| known.url != entry.url);
    entries.insert(0, entry);
    entries.truncate(HISTORY_LIMIT);
}

pub fn remove(entries: &mut Vec<HistoryEntry>, url: &str) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.url != url);
    entries.len() != before
}

pub fn title_of(entries: &[HistoryEntry], url: &str) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.url == url)
        .and_then(|entry| entry.title.clone())
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_LIMIT, HistoryEntry, record, remove, title_of};

    fn entry(id: &str) -> HistoryEntry {
        HistoryEntry {
            url: format!("https://www.youtube.com/watch?v={id:0>11}"),
            title: Some(format!("Song {id}")),
            duration_secs: Some(225),
        }
    }

    #[test]
    fn replayed_videos_move_to_the_top_without_duplicates() {
        let mut history = Vec::new();
        for id in ["a", "b", "c"] {
            record(&mut history, entry(id));
        }
        record(&mut history, entry("a"));
        let titles: Vec<_> = history.iter().map(|e| e.title.clone()).collect();
        assert_eq!(
            titles,
            [
                Some("Song a".into()),
                Some("Song c".into()),
                Some("Song b".into())
            ]
        );
    }

    #[test]
    fn keeps_only_the_newest_fifty() {
        let mut history = Vec::new();
        for index in 0..HISTORY_LIMIT + 5 {
            record(&mut history, entry(&index.to_string()));
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0], entry(&(HISTORY_LIMIT + 4).to_string()));
        assert_eq!(history[HISTORY_LIMIT - 1], entry("5"));
    }

    #[test]
    fn removes_one_entry_and_reuses_known_titles() {
        let mut history = Vec::new();
        record(&mut history, entry("a"));
        record(&mut history, entry("b"));
        assert_eq!(
            title_of(&history, &entry("a").url).as_deref(),
            Some("Song a")
        );
        assert!(remove(&mut history, &entry("a").url));
        assert!(!remove(&mut history, &entry("a").url));
        assert_eq!(history, [entry("b")]);
        assert_eq!(title_of(&history, &entry("a").url), None);
    }
}
