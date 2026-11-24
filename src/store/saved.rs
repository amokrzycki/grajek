use serde::{Deserialize, Serialize};

use crate::{
    destination::Destination,
    favorites::{self, FavoriteEntry},
    history::{self, HistoryEntry},
    preferences::{RepeatMode, clamp_volume},
    queue::{QUEUE_LIMIT, Queue},
    youtube::{safe_title, video_url},
};

pub const DEFAULT_VOLUME: u8 = 100;

/// Durable pending item: runtime IDs and expiring stream data never enter this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueueItem {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// One malformed item must not discard siblings, preferences, queue or history.
pub(crate) fn lenient_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(items) => items.into_iter().filter_map(lenient).collect(),
        _ => Vec::new(),
    })
}

fn lenient<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Option<T> {
    serde_json::from_value(value).ok()
}

fn read_destination<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Destination>, D::Error> {
    Ok(lenient(serde_json::Value::deserialize(deserializer)?))
}

const fn default_volume() -> u8 {
    DEFAULT_VOLUME
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub close_to_tray: bool,
    #[serde(default, deserialize_with = "read_destination")]
    pub destination: Option<Destination>,
    /// Percent, 0 to 100. Files without it (older versions) keep full volume.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Opt-in recovery for queued media failures; older files keep stop-on-error.
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default)]
    pub remember_queue: bool,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub queue: Vec<SavedQueueItem>,
    /// Newest first, unique by URL, at most [`crate::history::HISTORY_LIMIT`] entries.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Newest first, unique by canonical URL, at most [`favorites::FAVORITES_LIMIT`] entries.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub favorites: Vec<FavoriteEntry>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            close_to_tray: false,
            destination: None,
            volume: DEFAULT_VOLUME,
            continue_on_error: false,
            remember_queue: false,
            repeat: RepeatMode::Off,
            queue: Vec::new(),
            history: Vec::new(),
            favorites: Vec::new(),
        }
    }
}

impl Saved {
    pub fn restored_queue(&self) -> Queue {
        let mut queue = Queue::default();
        if self.remember_queue {
            for entry in &self.queue {
                if queue.push(entry.url.clone(), entry.title.clone()).is_err() {
                    break;
                }
            }
        }
        queue
    }

    /// The file is user-editable input: clamp what is out of range and drop what is not a
    /// playable video, rather than trusting it. Returns the number of valid unique
    /// favorites dropped beyond [`favorites::FAVORITES_LIMIT`].
    pub(super) fn sanitized(mut self) -> (Self, usize) {
        self.volume = clamp_volume(self.volume);
        let (favorites, dropped) = favorites::sanitize(self.favorites);
        let mut clean = Self {
            close_to_tray: self.close_to_tray,
            destination: self.destination,
            volume: self.volume,
            continue_on_error: self.continue_on_error,
            remember_queue: self.remember_queue,
            repeat: self.repeat,
            favorites,
            queue: if self.remember_queue {
                self.queue
                    .into_iter()
                    .filter_map(|entry| {
                        Some(SavedQueueItem {
                            url: video_url(&entry.url).ok()?,
                            title: entry.title.as_deref().and_then(safe_title),
                        })
                    })
                    .take(QUEUE_LIMIT)
                    .collect()
            } else {
                Vec::new()
            },
            history: Vec::new(),
        };
        for mut entry in self.history.into_iter().rev() {
            let Ok(url) = video_url(&entry.url) else {
                continue;
            };
            entry.url = url;
            entry.title = entry.title.as_deref().and_then(safe_title);
            history::record(&mut clean.history, entry);
        }
        (clean, dropped)
    }
}
