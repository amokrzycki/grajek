use super::*;
use crate::{
    favorites::{FAVORITES_LIMIT, FavoriteEntry},
    history,
    test_support::{Scratch, history_entry},
};

fn favorite(index: usize) -> FavoriteEntry {
    FavoriteEntry {
        title: Some(format!("Favorite {index}")),
        channel: Some("Channel".into()),
        duration_secs: Some(index as u64),
        ..crate::test_support::favorite(
            &format!("https://www.youtube.com/watch?v={index:0>11}"),
            None,
        )
    }
}

#[test]
fn legacy_files_load_with_empty_favorites() -> Result<()> {
    let saved: Saved = serde_json::from_str(
        r#"{"volume":37,"history":[{"url":"https://www.youtube.com/watch?v=aaaaaaaaaaa"}]}"#,
    )?;
    assert_eq!(saved.favorites.len(), 0);
    assert_eq!(saved.volume, 37);
    Ok(())
}

#[test]
fn missing_metadata_and_live_default_to_absent_or_false() -> anyhow::Result<()> {
    let saved: Saved = serde_json::from_value(serde_json::json!({
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
    let saved: Saved = serde_json::from_value(serde_json::json!({
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
    let saved: Saved = serde_json::from_str(r#"{"volume":42,"favorites":{"url":"x"}}"#)?;
    assert_eq!(saved.volume, 42);
    assert_eq!(saved.favorites.len(), 0);
    Ok(())
}

#[test]
fn favorites_round_trip() -> Result<()> {
    let scratch = Scratch::new()?;
    let saved = Saved {
        volume: 35,
        favorites: vec![
            FavoriteEntry {
                url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
                title: Some("First".into()),
                channel: Some("A channel".into()),
                duration_secs: Some(99),
                live: false,
            },
            FavoriteEntry {
                url: "https://www.youtube.com/watch?v=bbbbbbbbbbb".into(),
                title: None,
                channel: None,
                duration_secs: None,
                live: true,
            },
        ],
        ..Saved::default()
    };
    write(&scratch.file("grajek/state.json"), &saved)?;
    let loaded = load(Some(scratch.file("grajek/state.json")));
    ensure!(loaded.warning.is_none());
    assert_eq!(loaded.saved, saved);
    Ok(())
}

#[test]
fn oversized_favorites_are_backed_up_and_bounded() -> Result<()> {
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
    let mut saved = Saved {
        volume: 42,
        remember_queue: true,
        ..Saved::default()
    };
    history::record(&mut saved.history, history_entry("a"));
    saved.queue = vec![crate::store::SavedQueueItem {
        url: history_entry("b").url,
        title: Some("Queued".into()),
    }];
    saved.favorites = (0..FAVORITES_LIMIT + 5).map(favorite).collect();
    write(&file, &saved)?;
    let original = fs::read(&file)?;

    let loaded = load(Some(file.clone()));
    assert_eq!(loaded.saved.favorites.len(), FAVORITES_LIMIT);
    assert_eq!(loaded.saved.volume, 42);
    assert_eq!(loaded.saved.history.len(), 1);
    assert_eq!(loaded.saved.queue.len(), 1);
    assert!(loaded.path.is_some());
    assert!(
        loaded
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("favorites"))
    );

    let backups: Vec<_> = names(file.parent().context("no parent")?)?
        .into_iter()
        .filter(|name| name.contains("favorites-"))
        .collect();
    assert_eq!(backups.len(), 1, "one exact backup is preserved");
    assert_eq!(
        fs::read(file.parent().context("no parent")?.join(&backups[0]))?,
        original
    );

    write(&file, &loaded.saved)?;
    let reloaded = load(Some(file));
    assert_eq!(reloaded.saved.favorites.len(), FAVORITES_LIMIT);
    assert!(reloaded.warning.is_none());
    assert_eq!(reloaded.saved.volume, 42);
    assert_eq!(reloaded.saved.history.len(), 1);
    assert_eq!(reloaded.saved.queue.len(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn failed_favorites_backup_disables_saving_and_preserves_original() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
    let dir = file.parent().context("no parent")?.to_path_buf();
    fs::create_dir_all(&dir)?;
    let mut saved = Saved {
        volume: 7,
        ..Saved::default()
    };
    saved.favorites = (0..=FAVORITES_LIMIT).map(favorite).collect();
    write(&file, &saved)?;
    let original = fs::read(&file)?;

    let restore = fs::metadata(&dir)?.permissions();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500))?;
    // Running as root ignores the directory mode; skip rather than assert a false failure.
    if std::fs::File::create_new(dir.join("probe")).is_ok() {
        fs::set_permissions(&dir, restore)?;
        return Ok(());
    }
    let loaded = load(Some(file.clone()));
    fs::set_permissions(&dir, restore)?;

    assert!(loaded.path.is_none());
    assert!(
        loaded
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("favorites"))
    );
    assert_eq!(loaded.saved.favorites.len(), FAVORITES_LIMIT);
    assert_eq!(fs::read(&file)?, original, "original must be untouched");
    Ok(())
}

#[tokio::test]
async fn saver_flushes_favorites_on_close() -> Result<()> {
    let scratch = Scratch::new()?;
    let (sender, receiver) = watch::channel(Saved::default());
    let saver = spawn_saver(scratch.file("grajek/state.json"), receiver);
    let last = Saved {
        volume: 9,
        favorites: vec![FavoriteEntry {
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("Kept".into()),
            channel: Some("Channel".into()),
            duration_secs: Some(30),
            live: true,
        }],
        ..Saved::default()
    };
    sender.send_replace(last.clone());
    drop(sender);
    saver.task.await?;
    assert_eq!(load(Some(scratch.file("grajek/state.json"))).saved, last);
    Ok(())
}

#[test]
fn failed_write_preserves_prior_favorites_data() -> Result<()> {
    let scratch = Scratch::new()?;
    let file = scratch.file("grajek/state.json");
    let saved = Saved {
        favorites: vec![FavoriteEntry {
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("Kept".into()),
            channel: None,
            duration_secs: None,
            live: false,
        }],
        ..Saved::default()
    };
    write(&file, &saved)?;
    let dir = file.parent().context("no parent")?.to_path_buf();
    let backup = scratch.file("dir-as-file");
    fs::rename(&dir, &backup)?;
    fs::write(&dir, b"block")?;
    assert!(write(&file, &Saved::default()).is_err());
    fs::remove_file(&dir)?;
    fs::rename(&backup, &dir)?;
    assert_eq!(load(Some(file)).saved, saved);
    Ok(())
}
