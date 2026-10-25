//! The small JSON file that carries playback preferences and history across restarts.

use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, DirBuilder, File},
    io::{self, Write as _},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::watch, task::JoinHandle};

use crate::{
    config::video_url,
    player::RepeatMode,
    queue::{QUEUE_LIMIT, Queue},
};

pub const HISTORY_LIMIT: usize = 50;
pub const DEFAULT_VOLUME: u8 = 100;
const MAX_TITLE_CHARS: usize = 200;

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

/// Durable pending item: runtime IDs and expiring stream data never enter this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueueItem {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
}

// One malformed item must not throw away unrelated preferences or the rest of the FIFO.
fn read_queue<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<SavedQueueItem>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

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

const fn default_volume() -> u8 {
    DEFAULT_VOLUME
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
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
    #[serde(default, deserialize_with = "read_queue")]
    pub queue: Vec<SavedQueueItem>,
    /// Newest first, unique by URL, at most [`HISTORY_LIMIT`] entries.
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            continue_on_error: false,
            remember_queue: false,
            repeat: RepeatMode::Off,
            queue: Vec::new(),
            history: Vec::new(),
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

    /// Moves a replayed video to the top instead of duplicating it.
    pub fn record(&mut self, entry: HistoryEntry) {
        self.history.retain(|known| known.url != entry.url);
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
    }

    pub fn remove(&mut self, url: &str) -> bool {
        let before = self.history.len();
        self.history.retain(|entry| entry.url != url);
        self.history.len() != before
    }

    pub fn title_of(&self, url: &str) -> Option<String> {
        self.history
            .iter()
            .find(|entry| entry.url == url)
            .and_then(|entry| entry.title.clone())
    }

    /// The file is user-editable input: clamp what is out of range and drop what is not a
    /// playable video, rather than trusting it.
    fn sanitized(mut self) -> Self {
        self.volume = self.volume.min(100);
        let mut clean = Self {
            volume: self.volume,
            continue_on_error: self.continue_on_error,
            remember_queue: self.remember_queue,
            repeat: self.repeat,
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
            clean.record(entry);
        }
        clean
    }
}

/// `$XDG_DATA_HOME/grajek`, else `$HOME/.local/share/grajek`. Relative XDG values are
/// invalid per the specification and ignored.
#[cfg(any(target_os = "linux", test))]
pub fn data_dir(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("grajek"))
}

pub struct Loaded {
    pub saved: Saved,
    /// `None` disables saving, e.g. when no data directory exists or a damaged file
    /// could not be set aside.
    pub path: Option<PathBuf>,
    pub warning: Option<String>,
}

#[cfg(any(target_os = "windows", test))]
pub fn windows_data_dir(local: Option<OsString>) -> Option<PathBuf> {
    local
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Grajek"))
}

pub fn app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_data_dir(std::env::var_os("LOCALAPPDATA"))
    }
    #[cfg(target_os = "linux")]
    {
        data_dir(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
    }
}

pub fn open() -> Loaded {
    let dir = app_data_dir();
    load(dir.map(|dir| dir.join("state.json")))
}

pub fn load(path: Option<PathBuf>) -> Loaded {
    let Some(file) = path else {
        return Loaded {
            saved: Saved::default(),
            path: None,
            warning: Some(
                "No data directory found; history and settings will not be saved.".into(),
            ),
        };
    };
    let text = match fs::read(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                saved: Saved::default(),
                path: Some(file),
                warning: None,
            };
        }
        Err(error) => {
            tracing::warn!(%error, path = %file.display(), "Could not read saved state");
            return Loaded {
                saved: Saved::default(),
                path: None,
                warning: Some(format!(
                    "Could not read {}; history and settings will not be saved.",
                    file.display()
                )),
            };
        }
    };
    match serde_json::from_slice::<Saved>(&text) {
        Ok(saved) => Loaded {
            saved: saved.sanitized(),
            path: Some(file),
            warning: None,
        },
        Err(error) => {
            tracing::warn!(%error, path = %file.display(), "Saved state is malformed");
            // Keep the damaged file for inspection; the next save must not replace it.
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let backup = file.with_extension(format!("json.corrupt-{seconds}"));
            let kept = fs::rename(&file, &backup).is_ok();
            let warning = if kept {
                format!(
                    "Saved history was unreadable; starting empty. The old file is kept as {}.",
                    backup.display()
                )
            } else {
                format!(
                    "Saved history at {} is unreadable and could not be set aside; it will not be overwritten.",
                    file.display()
                )
            };
            Loaded {
                saved: Saved::default(),
                path: kept.then_some(file),
                warning: Some(warning),
            }
        }
    }
}

/// Writes through a temporary file in the same directory and renames it into place, so a
/// crash leaves either the old file or the new one.
pub fn write(path: &Path, saved: &Saved) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("state path has no directory"))?;
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(saved)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// The only writer. It always saves the latest snapshot and skips intermediate ones, so a
/// slow write can never land after, and overwrite, newer state. It exits (after saving
/// the final snapshot) when the sender is dropped.
pub fn spawn_saver(path: PathBuf, mut snapshots: watch::Receiver<Saved>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while snapshots.changed().await.is_ok() {
            let saved = snapshots.borrow_and_update().clone();
            let target = path.clone();
            match tokio::task::spawn_blocking(move || write(&target, &saved)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "Could not save application state"),
                Err(error) => tracing::warn!(%error, "Saving application state was interrupted"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_VOLUME, HISTORY_LIMIT, HistoryEntry, Saved, data_dir, load, spawn_saver, write,
    };
    use anyhow::{Context, Result, ensure};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::watch;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Result<Self> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "grajek-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
        fn file(&self) -> PathBuf {
            self.0.join("grajek").join("state.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(id: &str) -> HistoryEntry {
        HistoryEntry {
            url: format!("https://www.youtube.com/watch?v={id:0>11}"),
            title: Some(format!("Song {id}")),
            duration_secs: Some(225),
        }
    }

    fn names(dir: &Path) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for item in fs::read_dir(dir)? {
            names.push(item?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    #[test]
    fn defaults_to_full_volume_and_empty_history() {
        let saved = Saved::default();
        assert_eq!(saved.volume, DEFAULT_VOLUME);
        assert_eq!(saved.history.len(), 0);
    }

    #[test]
    fn replayed_videos_move_to_the_top_without_duplicates() {
        let mut saved = Saved::default();
        for id in ["a", "b", "c"] {
            saved.record(entry(id));
        }
        saved.record(entry("a"));
        let titles: Vec<_> = saved.history.iter().map(|e| e.title.clone()).collect();
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
        let mut saved = Saved::default();
        for index in 0..HISTORY_LIMIT + 5 {
            saved.record(entry(&index.to_string()));
        }
        assert_eq!(saved.history.len(), HISTORY_LIMIT);
        assert_eq!(saved.history[0], entry(&(HISTORY_LIMIT + 4).to_string()));
        assert_eq!(saved.history[HISTORY_LIMIT - 1], entry("5"));
    }

    #[test]
    fn removes_one_entry_and_reuses_known_titles() {
        let mut saved = Saved::default();
        saved.record(entry("a"));
        saved.record(entry("b"));
        assert_eq!(saved.title_of(&entry("a").url).as_deref(), Some("Song a"));
        assert!(saved.remove(&entry("a").url));
        assert!(!saved.remove(&entry("a").url));
        assert_eq!(saved.history, [entry("b")]);
        assert_eq!(saved.title_of(&entry("a").url), None);
    }

    #[test]
    fn data_dir_follows_the_xdg_rules() {
        let root = std::env::temp_dir();
        let home = root.join("home-u");
        let xdg = root.join("xdg-data");
        let home_os = Some(home.clone().into_os_string());
        assert_eq!(
            data_dir(Some(xdg.clone().into_os_string()), home_os.clone()),
            Some(xdg.join("grajek"))
        );
        for xdg in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                data_dir(xdg, home_os.clone()),
                Some(home.join(".local/share/grajek"))
            );
        }
        assert_eq!(data_dir(None, Some("".into())), None);
        assert_eq!(data_dir(None, None), None);
        assert_eq!(
            super::windows_data_dir(Some(root.clone().into_os_string())),
            Some(root.join("Grajek"))
        );
        for value in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(super::windows_data_dir(value), None);
        }
    }

    #[test]
    fn failed_directory_creation_or_replacement_preserves_existing_data() -> Result<()> {
        let scratch = Scratch::new()?;
        let blocked = scratch.0.join("blocked");
        fs::write(&blocked, b"keep me")?;
        assert!(write(&blocked.join("state.json"), &Saved::default()).is_err());
        assert_eq!(fs::read(&blocked)?, b"keep me");
        let destination = scratch.file();
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("keep"), b"original")?;
        assert!(write(&destination, &Saved::default()).is_err());
        assert_eq!(fs::read(destination.join("keep"))?, b"original");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn locked_windows_destination_is_retained_and_can_be_replaced_after_close() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt as _;
        let scratch = Scratch::new()?;
        let path = scratch.file();
        write(&path, &Saved::default())?;
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)?;
        let saved = Saved {
            volume: 27,
            ..Saved::default()
        };
        assert!(write(&path, &saved).is_err());
        drop(held);
        assert_eq!(load(Some(path.clone())).saved, Saved::default());
        write(&path, &saved)?;
        assert_eq!(load(Some(path)).saved, saved);
        Ok(())
    }

    #[test]
    fn missing_file_starts_empty_without_a_warning() -> Result<()> {
        let scratch = Scratch::new()?;
        let loaded = load(Some(scratch.file()));
        assert_eq!(loaded.saved, Saved::default());
        assert!(loaded.warning.is_none() && loaded.path.is_some());
        Ok(())
    }

    #[test]
    fn saved_state_round_trips() -> Result<()> {
        let scratch = Scratch::new()?;
        let mut saved = Saved {
            volume: 35,
            continue_on_error: true,
            remember_queue: true,
            repeat: crate::player::RepeatMode::Queue,
            queue: vec![
                super::SavedQueueItem {
                    url: entry("a").url,
                    title: Some("Queued".into())
                };
                2
            ],
            ..Saved::default()
        };
        saved.record(entry("a"));
        saved.record(HistoryEntry {
            title: None,
            duration_secs: None,
            ..entry("b")
        });
        write(&scratch.file(), &saved)?;
        let loaded = load(Some(scratch.file()));
        ensure!(loaded.warning.is_none());
        assert_eq!(loaded.saved, saved);
        // The temporary file does not linger after a successful write.
        assert_eq!(names(&scratch.file().with_file_name(""))?, ["state.json"]);
        Ok(())
    }

    #[test]
    fn writes_replace_the_file_and_ignore_a_leftover_temporary() -> Result<()> {
        let scratch = Scratch::new()?;
        let file = scratch.file();
        write(&file, &Saved::default())?;
        // A crash mid-write leaves a partial temporary file; the real file stays valid.
        fs::write(file.with_extension("json.tmp"), b"{\"volume\": 4")?;
        assert_eq!(load(Some(file.clone())).saved, Saved::default());
        let changed = Saved {
            volume: 7,
            ..Saved::default()
        };
        write(&file, &changed)?;
        assert_eq!(load(Some(file)).saved, changed);
        Ok(())
    }

    #[test]
    fn write_failure_keeps_the_previous_file_and_can_be_retried() -> Result<()> {
        let scratch = Scratch::new()?;
        let file = scratch.file();
        write(&file, &Saved::default())?;
        fs::create_dir(file.with_extension("json.tmp"))?;
        let changed = Saved {
            remember_queue: true,
            volume: 25,
            ..Saved::default()
        };
        assert!(write(&file, &changed).is_err());
        assert_eq!(load(Some(file.clone())).saved, Saved::default());
        fs::remove_dir(file.with_extension("json.tmp"))?;
        write(&file, &changed)?;
        assert_eq!(load(Some(file)).saved, changed);
        Ok(())
    }

    #[test]
    fn malformed_file_is_kept_and_reported() -> Result<()> {
        let scratch = Scratch::new()?;
        let file = scratch.file();
        fs::create_dir_all(file.parent().context("no parent")?)?;
        fs::write(&file, b"{ not json")?;
        let loaded = load(Some(file.clone()));
        assert_eq!(loaded.saved, Saved::default());
        assert!(loaded.warning.is_some_and(|w| w.contains("unreadable")));
        // The damaged bytes survive under another name and saving may resume.
        ensure!(loaded.path.is_some() && !file.exists());
        let kept = names(file.parent().context("no parent")?)?;
        assert_eq!(kept.len(), 1);
        assert!(kept[0].starts_with("state.json.corrupt-"));
        assert_eq!(fs::read(file.with_file_name(&kept[0]))?, b"{ not json");
        Ok(())
    }

    #[test]
    fn unusable_values_are_repaired_not_trusted() -> Result<()> {
        let scratch = Scratch::new()?;
        let file = scratch.file();
        fs::create_dir_all(file.parent().context("no parent")?)?;
        let a = entry("a");
        let json = format!(
            r#"{{"volume":250,"history":[
                {{"url":"https://youtu.be/{id}","title":"Short link"}},
                {{"url":"https://evil.example/x"}},
                {{"url":"{url}","title":"Duplicate"}},
                {{"url":"not a url"}}
            ]}}"#,
            id = "00000000a",
            url = a.url
        );
        fs::write(&file, json)?;
        let loaded = load(Some(file));
        assert_eq!(loaded.saved.volume, 100);
        assert_eq!(loaded.saved.history.len(), 1);
        assert_eq!(loaded.saved.history[0].url, a.url);
        Ok(())
    }

    #[test]
    fn files_without_a_volume_keep_full_volume() -> Result<()> {
        let scratch = Scratch::new()?;
        let file = scratch.file();
        fs::create_dir_all(file.parent().context("no parent")?)?;
        fs::write(&file, br#"{"history":[]}"#)?;
        assert_eq!(load(Some(file)).saved.volume, DEFAULT_VOLUME);
        Ok(())
    }

    #[test]
    fn missing_data_directory_disables_saving() {
        let loaded = load(None);
        assert!(loaded.path.is_none() && loaded.warning.is_some());
    }

    #[tokio::test]
    async fn saver_writes_the_latest_snapshot_and_flushes_on_exit() -> Result<()> {
        let scratch = Scratch::new()?;
        let (sender, receiver) = watch::channel(Saved::default());
        let saver = spawn_saver(scratch.file(), receiver);
        let mut last = Saved::default();
        for volume in 0..20 {
            last = Saved {
                volume,
                remember_queue: true,
                repeat: crate::player::RepeatMode::Track,
                queue: vec![super::SavedQueueItem {
                    url: entry(&volume.to_string()).url,
                    title: None,
                }],
                ..Saved::default()
            };
            sender.send_replace(last.clone());
        }
        drop(sender);
        saver.await?;
        assert_eq!(load(Some(scratch.file())).saved, last);
        Ok(())
    }

    #[test]
    fn old_state_keeps_history_and_volume_with_recovery_disabled() -> Result<()> {
        let saved: Saved = serde_json::from_str(
            r#"{"volume":37,"history":[{"url":"https://www.youtube.com/watch?v=aaaaaaaaaaa","title":"Kept"}]}"#,
        )?;
        assert!(!saved.continue_on_error);
        assert_eq!(saved.volume, 37);
        assert_eq!(saved.history[0].title.as_deref(), Some("Kept"));
        Ok(())
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use crate::{player::RepeatMode, queue::QUEUE_LIMIT};
    use anyhow::Result;

    #[test]
    fn old_files_default_to_no_queue_and_repeat_off() -> Result<()> {
        let saved: Saved =
            serde_json::from_str(r#"{"volume":37,"continue_on_error":true,"history":[]}"#)?;
        assert!(!saved.remember_queue);
        assert_eq!(saved.repeat, RepeatMode::Off);
        assert_eq!(saved.queue.len(), 0);
        assert_eq!(saved.volume, 37);
        assert!(saved.continue_on_error);
        Ok(())
    }

    #[test]
    fn queue_loading_is_bounded_preserves_duplicates_and_isolates_bad_entries() -> Result<()> {
        let entries = serde_json::json!([
            null, {"title":"missing URL"}, {"url":42},
            {"url":"https://evil.example/stream?token=secret"},
            {"url":"https://youtu.be/aaaaaaaaaaa", "title":"  Hello\nworld\u{0000}  "},
            {"url":"https://www.youtube.com/watch?v=aaaaaaaaaaa", "title":"x".repeat(500)},
            {"url":"https://youtu.be/bbbbbbbbbbb", "title":false}
        ]);
        let saved: Saved = serde_json::from_value(serde_json::json!({
            "remember_queue":true, "repeat":"queue", "queue":entries, "volume":42
        }))?;
        let saved = saved.sanitized();
        assert_eq!(saved.queue.len(), 2);
        assert_eq!(saved.queue[0].url, saved.queue[1].url);
        assert_eq!(
            saved.queue[0].url,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa"
        );
        assert_eq!(saved.queue[0].title.as_deref(), Some("Hello world"));
        assert_eq!(
            saved.queue[1].title.as_ref().map(|t| t.chars().count()),
            Some(200)
        );
        assert_eq!(saved.repeat, RepeatMode::Queue);
        let mut huge = saved.clone();
        huge.queue = vec![saved.queue[0].clone(); QUEUE_LIMIT + 5];
        assert_eq!(huge.sanitized().queue.len(), QUEUE_LIMIT);
        let mut disabled = saved;
        disabled.remember_queue = false;
        assert_eq!(disabled.sanitized().queue.len(), 0);
        Ok(())
    }

    #[test]
    fn malformed_queue_field_and_unknown_repeat_preserve_other_preferences() -> Result<()> {
        let saved: Saved = serde_json::from_str(
            r#"{"volume":45,"remember_queue":true,"repeat":"future","queue":{}}"#,
        )?;
        assert_eq!(saved.volume, 45);
        assert_eq!(saved.repeat, RepeatMode::Off);
        assert_eq!(saved.queue.len(), 0);
        Ok(())
    }
}
