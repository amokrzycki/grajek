use super::*;
use std::{fs, path::PathBuf};

struct Storage(PathBuf);

impl Drop for Storage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn runtime_save_warning_precedes_long_startup_notices() {
    let (mut controller, _fake, updates) = solo(Saved::default());
    let startup = "Recovered history file with a very long storage path. ".repeat(20);
    controller.notice = Some(startup.clone());
    let (status, changes) = watch::channel(Some("Could not save history and settings."));
    controller.saver_status = Some(changes);
    controller.sync();
    let warning = updates.borrow().notice.clone().unwrap_or_default();
    assert!(warning.starts_with("Could not save history and settings."));
    assert!(warning.contains(&startup));
    status.send_replace(None);
    controller.sync();
    assert_eq!(updates.borrow().notice.as_deref(), Some(startup.as_str()));
}

#[tokio::test]
async fn save_failure_and_recovery_reach_playback_without_erasing_startup_notices() -> Result<()> {
    let directory =
        std::env::temp_dir().join(format!("grajek-saver-notice-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let storage = Storage(directory);
    let path = storage.0.join("state.json");
    let initial = Saved {
        remember_queue: true,
        queue: vec![SavedQueueItem {
            url: B.into(),
            title: Some("Pending".into()),
        }],
        ..Saved::default()
    };
    crate::store::write(&path, &initial)?;
    let loaded = crate::store::load(Some(path.clone()));
    let fake = Fake::new();
    let (commands, receiver) = mpsc::channel(16);
    let (_volume, volume) = watch::channel(VolumeRequest::default());
    let (states, mut updates) = watch::channel(State::default());
    let (persist, snapshots) = watch::channel(loaded.saved.clone());
    let saver = crate::store::spawn_saver(path.clone(), snapshots);
    let (shutdown, stopping) = watch::channel(false);
    let controller = Controller::new(
        fake.clone(),
        loaded.saved,
        Some("Startup warning.".into()),
        states,
        persist,
        Some(saver.status),
    );
    let task = tokio::spawn(controller.run(receiver, volume, stopping));
    commands.send(Command::Play(A.into())).await?;
    eventually(|| fake.started() == 1).await?;
    fake.track(0)?.advance(1);
    timeout(
        Duration::from_secs(5),
        updates.wait_for(|state| state.phase == Phase::Playing),
    )
    .await??;

    let backup = storage.0.join("previous.json");
    fs::rename(&path, &backup)?;
    fs::create_dir(&path)?;
    commands.send(Command::CloseToTray(true)).await?;
    let failed = timeout(
        Duration::from_secs(5),
        updates.wait_for(|state| {
            state
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Could not save"))
        }),
    )
    .await??
    .clone();
    assert_eq!(failed.phase, Phase::Playing);
    assert_eq!(failed.queue[0].url, B);
    assert!(
        failed
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("Startup warning."))
    );

    fs::remove_dir(&path)?;
    fs::rename(&backup, &path)?;
    commands.send(Command::CloseToTray(false)).await?;
    timeout(
        Duration::from_secs(5),
        updates.wait_for(|state| state.notice.as_deref() == Some("Startup warning.")),
    )
    .await??;
    assert_eq!(updates.borrow().phase, Phase::Playing);
    shutdown.send_replace(true);
    task.await?;
    saver.task.await?;
    assert!(!crate::store::load(Some(path)).saved.close_to_tray);
    Ok(())
}
