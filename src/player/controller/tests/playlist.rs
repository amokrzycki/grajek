use super::*;
use crate::playlist::{Entry, Playlist};

const LIST: &str = "https://www.youtube.com/playlist?list=PL_test123";

fn metadata(urls: &[&str]) -> Playlist {
    Playlist {
        entries: urls
            .iter()
            .map(|url| Entry {
                url: (*url).into(),
                title: Some(format!("Title {url}")),
            })
            .collect(),
        unavailable: 0,
        limited: false,
    }
}

async fn start(rig: &mut Rig) -> Result<oneshot::Sender<Result<Playlist>>> {
    let reply = rig.fake.import_reply();
    rig.send(Command::ImportPlaylist(LIST.into())).await?;
    rig.until(|state| state.import == ImportStatus::Importing)
        .await?;
    eventually(|| rig.fake.playlist_calls.load(SeqCst) > 0).await?;
    Ok(reply)
}

fn complete(reply: oneshot::Sender<Result<Playlist>>, playlist: Playlist) -> Result<()> {
    reply
        .send(Ok(playlist))
        .map_err(|_| anyhow::anyhow!("import was cancelled"))
}

#[tokio::test]
async fn import_is_atomic_ordered_and_reuses_titles_without_voice_or_playback() -> Result<()> {
    let mut rig = Rig::new().await?;
    let reply = start(&mut rig).await?;
    assert_eq!(rig.queued(), Vec::<String>::new());
    complete(reply, metadata(&[A, B, A]))?;
    let state = rig
        .until(|s| matches!(s.import, ImportStatus::Finished { .. }))
        .await?;
    assert_eq!(rig.queued(), [A, B, A]);
    assert_eq!(state.phase, Phase::Ready);
    assert_eq!(
        state
            .queue
            .iter()
            .map(|item| item.id)
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    assert!(state.queue.iter().all(|item| item.title.is_some()));
    assert_eq!(rig.fake.title_requests(), Vec::<String>::new());
    assert_eq!(rig.fake.requests(), Vec::<String>::new());
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    assert_eq!(rig.fake.started(), 0);
    assert!(rig.saves.borrow().queue.is_empty(), "remember is off");
    Ok(())
}

#[tokio::test]
async fn capacity_is_rechecked_after_queue_edits_and_full_import_never_persists() -> Result<()> {
    for (initial, added_while_running, expected) in
        [(0, 0, 100), (75, 0, 25), (97, 2, 1), (99, 1, 0)]
    {
        let mut rig = Rig::start(Saved {
            remember_queue: true,
            queue: (0..initial)
                .map(|_| SavedQueueItem {
                    url: A.into(),
                    title: Some("Existing".into()),
                })
                .collect(),
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        let reply = start(&mut rig).await?;
        for _ in 0..added_while_running {
            rig.enqueue(B).await?;
        }
        rig.until(|s| s.queue.len() == initial + added_while_running)
            .await?;
        let mut batch = metadata(&vec![C; 100]);
        batch.limited = true;
        let saved_before = rig.saves.borrow().clone();
        rig.saves.borrow_and_update();
        complete(reply, batch)?;
        let state = rig.until(|s| s.import != ImportStatus::Importing).await?;
        assert_eq!(state.queue.len(), initial + added_while_running + expected);
        assert!(
            state
                .queue
                .iter()
                .skip(initial + added_while_running)
                .all(|item| item.url == C)
        );
        if expected == 0 {
            assert!(
                matches!(state.import, ImportStatus::Failed(ref message) if message.contains("Queue full"))
            );
            assert_eq!(*rig.saves.borrow(), saved_before);
            assert!(!rig.saves.has_changed()?);
        } else {
            assert_eq!(
                state.import,
                ImportStatus::Finished {
                    added: expected,
                    unavailable: 0,
                    omitted: 100 - expected,
                    limited: true
                }
            );
            assert_eq!(rig.saves.borrow().queue.len(), state.queue.len());
        }
    }
    Ok(())
}

#[tokio::test]
async fn cancel_clear_shutdown_and_second_request_cannot_commit_stale_data() -> Result<()> {
    for action in [Command::CancelImport, Command::ClearQueue] {
        let mut rig = Rig::new().await?;
        let reply = start(&mut rig).await?;
        rig.saves.borrow_and_update();
        rig.send(Command::ImportPlaylist(LIST.into())).await?;
        rig.send(action).await?;
        rig.until(|s| s.import == ImportStatus::Cancelled).await?;
        assert_eq!(rig.fake.playlist_calls.load(SeqCst), 1);
        assert!(reply.is_closed());
        assert!(reply.send(Ok(metadata(&[A]))).is_err());
        assert_eq!(rig.queued(), Vec::<String>::new());
        // Clear Queue itself persists its explicit edit; Cancel Import never does.
        let second = rig.fake.import_reply();
        rig.send(Command::ImportPlaylist(LIST.into())).await?;
        rig.until(|s| s.import == ImportStatus::Importing).await?;
        complete(second, metadata(&[B]))?;
        rig.until(|s| matches!(s.import, ImportStatus::Finished { .. }))
            .await?;
        assert_eq!(rig.queued(), [B]);
    }
    let mut rig = Rig::new().await?;
    let reply = start(&mut rig).await?;
    rig.shutdown.send_replace(true);
    rig.task.await?;
    assert!(reply.is_closed());
    assert!(rig.fake.shut_down.load(SeqCst));
    Ok(())
}

#[tokio::test]
async fn cancelled_and_failed_imports_never_write_a_queue_snapshot() -> Result<()> {
    let mut rig = Rig::new().await?;
    let reply = start(&mut rig).await?;
    rig.saves.borrow_and_update();
    rig.send(Command::CancelImport).await?;
    rig.until(|s| s.import == ImportStatus::Cancelled).await?;
    assert!(reply.is_closed());
    assert!(!rig.saves.has_changed()?);
    let reply = start(&mut rig).await?;
    reply
        .send(Err(crate::playlist::Error::Malformed.into()))
        .map_err(|_| anyhow::anyhow!("closed"))?;
    let state = rig
        .until(|s| matches!(s.import, ImportStatus::Failed(_)))
        .await?;
    assert!(state.import.message().contains("invalid playlist metadata"));
    assert!(!rig.saves.has_changed()?);
    assert_eq!(rig.queued(), Vec::<String>::new());
    rig.send(Command::ImportPlaylist("https://evil.invalid/".into()))
        .await?;
    rig.until(
        |s| matches!(&s.import, ImportStatus::Failed(message) if message.contains("playlist URL")),
    )
    .await?;
    assert_eq!(rig.fake.playlist_calls.load(SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn import_appends_after_latest_reorder_and_removal_then_persists_and_restores() -> Result<()>
{
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        repeat: RepeatMode::Queue,
        continue_on_error: true,
        queue: [A, B, C]
            .into_iter()
            .map(|url| SavedQueueItem {
                url: url.into(),
                title: Some("Existing".into()),
            })
            .collect(),
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    let ids: Vec<_> = rig.state().queue.iter().map(|item| item.id).collect();
    let reply = start(&mut rig).await?;
    rig.send(Command::Shift {
        id: ids[2],
        up: true,
    })
    .await?;
    rig.send(Command::Dequeue(ids[0])).await?;
    rig.until(|s| s.queue.len() == 2).await?;
    complete(
        reply,
        Playlist {
            unavailable: 3,
            ..metadata(&[A, A])
        },
    )?;
    let state = rig
        .until(|s| matches!(s.import, ImportStatus::Finished { .. }))
        .await?;
    assert_eq!(rig.queued(), [C, B, A, A]);
    assert!(state.import.message().contains("3 entries unavailable"));
    assert_eq!(state.preferences.repeat, RepeatMode::Queue);
    assert!(state.preferences.continue_on_error);
    let saved: Saved = serde_json::from_str(&serde_json::to_string(&*rig.saves.borrow())?)?;
    let mut restored = Rig::start(saved);
    restored.until(|s| s.phase == Phase::Ready).await?;
    assert_eq!(restored.queued(), [C, B, A, A]);
    assert_eq!(restored.fake.requests(), Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn playback_pause_volume_next_stop_and_repeat_remain_independent() -> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(5);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let reply = start(&mut rig).await?;
    rig.send(Command::Pause).await?;
    let before = rig.until(|s| s.phase == Phase::Paused).await?;
    rig.set_volume(35, false);
    rig.until(|s| s.volume == 35).await?;
    complete(reply, metadata(&[B, C]))?;
    let state = rig
        .until(|s| matches!(s.import, ImportStatus::Finished { .. }))
        .await?;
    assert_eq!(state.phase, Phase::Paused);
    assert_eq!(state.position, before.position);
    assert_eq!(rig.fake.started(), 1);
    rig.send(Command::Resume).await?;
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.track(0)?.end();
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [A, B]);
    assert_eq!(rig.queued(), [C, A]);
    let reply = start(&mut rig).await?;
    rig.send(Command::Next).await?;
    rig.started(3).await?;
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert!(!reply.is_closed());
    complete(reply, metadata(&[B]))?;
    rig.until(|s| matches!(s.import, ImportStatus::Finished { .. }))
        .await?;
    assert_eq!(rig.queued(), [A, B]);
    Ok(())
}

#[tokio::test]
async fn only_missing_import_titles_use_the_existing_shared_worker() -> Result<()> {
    let mut rig = Rig::new().await?;
    let title_reply = rig.fake.title_reply(B);
    let reply = start(&mut rig).await?;
    let mut batch = metadata(&[A, B, B]);
    batch.entries[1].title = None;
    batch.entries[2].title = None;
    complete(reply, batch)?;
    eventually(|| rig.fake.title_requests() == [B]).await?;
    title_reply
        .send(Ok(Some("Resolved title".into())))
        .map_err(|_| anyhow::anyhow!("closed"))?;
    rig.until(|s| s.queue.len() == 3 && s.queue.iter().all(|item| item.title.is_some()))
        .await?;
    assert_eq!(
        rig.state().queue[1].title.as_deref(),
        Some("Resolved title")
    );
    assert_eq!(rig.fake.title_requests(), [B]);
    Ok(())
}

#[tokio::test]
async fn dropping_the_controller_future_also_cancels_its_import() -> Result<()> {
    let mut rig = Rig::new().await?;
    let reply = start(&mut rig).await?;
    rig.task.abort();
    let _ = rig.task.await;
    eventually(|| reply.is_closed()).await?;
    Ok(())
}

#[tokio::test]
async fn already_full_queue_rejects_import_before_spawning() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        queue: (0..100)
            .map(|_| SavedQueueItem {
                url: A.into(),
                title: Some("Existing".into()),
            })
            .collect(),
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.saves.borrow_and_update();
    rig.send(Command::ImportPlaylist(LIST.into())).await?;
    rig.until(
        |s| matches!(&s.import,ImportStatus::Failed(message) if message.contains("Queue full")),
    )
    .await?;
    assert_eq!(rig.fake.playlist_calls.load(SeqCst), 0);
    assert_eq!(rig.queued().len(), 100);
    assert!(!rig.saves.has_changed()?);
    Ok(())
}
