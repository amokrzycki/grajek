use super::{DEFAULT_VOLUME, Saved, load, spawn_saver, write};
use crate::history::{self, HISTORY_LIMIT, HistoryEntry};
use crate::test_support::{Scratch, history_entry};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};
use tokio::sync::watch;

fn names(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for item in fs::read_dir(dir)? {
        names.push(item?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

#[test]
fn malformed_destination_keeps_existing_preferences_and_close_defaults_off() -> anyhow::Result<()> {
    let saved: Saved = serde_json::from_str(
        r#"{"volume":42,"remember_queue":true,"destination":{"token":"should-not-be-state"},"queue":[{"url":"https://youtu.be/BaW_jenozKc"}]}"#,
    )?;
    assert_eq!(saved.volume, 42);
    assert!(saved.destination.is_none());
    assert!(!saved.close_to_tray);
    assert_eq!(saved.queue.len(), 1);
    let json = serde_json::to_string(&saved)?;
    assert!(!json.contains("token"));
    assert!(!json.contains("should-not-be-state"));
    Ok(())
}

#[test]
fn defaults_to_full_volume_and_empty_history() {
    let saved = Saved::default();
    assert_eq!(saved.volume, DEFAULT_VOLUME);
    assert_eq!(saved.history.len(), 0);
}

#[test]
fn concurrent_writers_never_share_staging_files() -> Result<()> {
    use std::sync::{Arc, Barrier};
    let scratch = Scratch::new()?;
    let path = scratch.file("grajek/state.json");
    write(&path, &Saved::default())?;
    let barrier = Arc::new(Barrier::new(2));
    let mut large = Saved::default();
    for id in 0..HISTORY_LIMIT {
        let mut item = history_entry(&id.to_string());
        item.title = Some("ż".repeat(200));
        history::record(&mut large.history, item);
    }
    let small = Saved::default();
    let results = std::thread::scope(|scope| {
        let worker = |saved: &Saved| {
            let mut errors = 0;
            let mut corrupt = 0;
            for _ in 0..100 {
                barrier.wait();
                errors += usize::from(write(&path, saved).is_err());
                barrier.wait();
                corrupt += usize::from(
                    fs::read(&path)
                        .is_ok_and(|bytes| serde_json::from_slice::<Saved>(&bytes).is_err()),
                );
                barrier.wait();
            }
            (errors, corrupt)
        };
        let first = scope.spawn(move || worker(&large));
        let second = scope.spawn(move || worker(&small));
        Ok::<_, anyhow::Error>((
            first
                .join()
                .map_err(|_| anyhow::anyhow!("first writer panicked"))?,
            second
                .join()
                .map_err(|_| anyhow::anyhow!("second writer panicked"))?,
        ))
    })?;
    assert_eq!(
        results,
        ((0, 0), (0, 0)),
        "write failures and malformed snapshots"
    );
    assert_eq!(names(path.parent().context("no parent")?)?, ["state.json"]);
    Ok(())
}

#[test]
fn failed_directory_creation_or_replacement_preserves_existing_data() -> Result<()> {
    let scratch = Scratch::new()?;
    let blocked = scratch.file("blocked");
    fs::write(&blocked, b"keep me")?;
    assert!(write(&blocked.join("state.json"), &Saved::default()).is_err());
    assert_eq!(fs::read(&blocked)?, b"keep me");
    let destination = scratch.file("grajek/state.json");
    fs::create_dir_all(&destination)?;
    fs::write(destination.join("keep"), b"original")?;
    assert!(write(&destination, &Saved::default()).is_err());
    assert_eq!(fs::read(destination.join("keep"))?, b"original");
    assert_eq!(
        names(destination.parent().context("no parent")?)?,
        ["state.json"]
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn locked_windows_destination_is_retained_and_can_be_replaced_after_close() -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let scratch = Scratch::new()?;
    let path = scratch.file("grajek/state.json");
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
    let loaded = load(Some(scratch.file("grajek/state.json")));
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
        repeat: crate::preferences::RepeatMode::Queue,
        queue: vec![
            super::SavedQueueItem {
                url: history_entry("a").url,
                title: Some("Queued".into())
            };
            2
        ],
        ..Saved::default()
    };
    history::record(&mut saved.history, history_entry("a"));
    history::record(
        &mut saved.history,
        HistoryEntry {
            title: None,
            duration_secs: None,
            ..history_entry("b")
        },
    );
    write(&scratch.file("grajek/state.json"), &saved)?;
    let loaded = load(Some(scratch.file("grajek/state.json")));
    ensure!(loaded.warning.is_none());
    assert_eq!(loaded.saved, saved);
    // The temporary file does not linger after a successful write.
    assert_eq!(names(&scratch.file("grajek"))?, ["state.json"]);
    Ok(())
}

#[test]
fn writes_replace_the_file_and_ignore_a_leftover_temporary() -> Result<()> {
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
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
fn leftover_fixed_temporary_never_blocks_new_saves() -> Result<()> {
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
    write(&file, &Saved::default())?;
    fs::create_dir(file.with_extension("json.tmp"))?;
    let changed = Saved {
        remember_queue: true,
        volume: 25,
        ..Saved::default()
    };
    write(&file, &changed)?;
    assert_eq!(load(Some(file)).saved, changed);
    Ok(())
}

#[test]
fn malformed_file_is_kept_and_reported() -> Result<()> {
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
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
    let file = scratch.file("grajek/state.json");
    fs::create_dir_all(file.parent().context("no parent")?)?;
    let a = history_entry("a");
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
    let file = scratch.file("grajek/state.json");
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
    let saver = spawn_saver(scratch.file("grajek/state.json"), receiver);
    let mut last = Saved::default();
    for volume in 0..20 {
        last = Saved {
            volume,
            remember_queue: true,
            repeat: crate::preferences::RepeatMode::Track,
            queue: vec![super::SavedQueueItem {
                url: history_entry(&volume.to_string()).url,
                title: None,
            }],
            ..Saved::default()
        };
        sender.send_replace(last.clone());
    }
    drop(sender);
    saver.task.await?;
    assert_eq!(load(Some(scratch.file("grajek/state.json"))).saved, last);
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

mod favorites;

use crate::{preferences::RepeatMode, queue::QUEUE_LIMIT};

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
    let saved = saved.sanitized().0;
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
    assert_eq!(huge.sanitized().0.queue.len(), QUEUE_LIMIT);
    let mut disabled = saved;
    disabled.remember_queue = false;
    assert_eq!(disabled.sanitized().0.queue.len(), 0);
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
