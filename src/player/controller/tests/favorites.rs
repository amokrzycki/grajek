use super::*;
use crate::favorites::FAVORITES_LIMIT;
use crate::favorites::FavoriteRefusal;
use crate::search::{ResultItem, Results};
use crate::search::{SearchPhase, SearchState};
use crate::test_support::{Scratch, favorite, results};
use std::fs;

fn search_state(id: u64, results: Results) -> SearchState {
    SearchState {
        id,
        query: "q".into(),
        phase: SearchPhase::Results(results),
        status: None,
    }
}

fn favorite_ack(state: &State) -> Result<FavoriteResult> {
    state
        .favorite_result
        .clone()
        .context("favorite acknowledgement missing")
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one deterministic end-to-end check spans save, restore and removal"
)]
async fn the_cheapest_disproving_check_saves_and_restores_favorites() -> Result<()> {
    let mut saved = Saved::default();
    crate::history::record(
        &mut saved.history,
        HistoryEntry {
            url: B.into(),
            title: Some("Known Y".into()),
            duration_secs: Some(123),
        },
    );
    let mut rig = Rig::start(saved);
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.play(A).await?;
    rig.started(1).await?;
    let track = rig.fake.track(0)?;
    track.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;

    let track_id = rig.state().track_id;
    let position = rig.state().position;
    let phase = rig.state().phase;
    let seek = rig.state().seek;
    let repeat = rig.state().preferences.repeat;
    let requests = rig.fake.requests();
    let connects = rig.fake.connects.load(SeqCst);
    let disconnects = rig.fake.disconnects.load(SeqCst);
    let started = rig.fake.started();

    rig.send(Command::SetFavorite {
        id: 1,
        url: B.into(),
        saved: true,
    })
    .await?;
    rig.until(|s| {
        s.favorite_result
            .as_ref()
            .is_some_and(|r| r.request_id == 1)
    })
    .await?;
    rig.send(Command::SetFavorite {
        id: 2,
        url: B.into(),
        saved: true,
    })
    .await?;
    rig.until(|s| {
        s.favorite_result
            .as_ref()
            .is_some_and(|r| r.request_id == 2)
    })
    .await?;

    rig.send(Command::ClearHistory).await?;
    rig.send(Command::ClearQueue).await?;
    rig.send(Command::RememberQueue(false)).await?;
    rig.settle().await;

    let state = rig.state();
    assert_eq!(state.favorites.len(), 1, "exactly one favorite Y remains");
    assert_eq!(state.favorites[0].url, B);
    assert_eq!(state.favorites[0].title.as_deref(), Some("Known Y"));
    assert_eq!(state.favorites[0].duration_secs, Some(123));

    assert_eq!(rig.fake.requests(), requests);
    assert_eq!(rig.fake.connects.load(SeqCst), connects);
    assert_eq!(rig.fake.disconnects.load(SeqCst), disconnects);
    assert_eq!(rig.fake.started(), started);
    assert_eq!(rig.state().track_id, track_id);
    assert_eq!(rig.state().position, position);
    assert_eq!(rig.state().phase, phase);
    assert_eq!(rig.state().seek, seek);
    assert_eq!(rig.state().preferences.repeat, repeat);
    ensure!(
        !track.stopped.load(SeqCst),
        "X remains active in the original controller"
    );

    let scratch = Scratch::new()?;
    crate::store::write(&scratch.file("state.json"), &rig.saves.borrow())?;
    let reloaded = crate::store::load(Some(scratch.file("state.json")));
    assert_eq!(reloaded.saved.favorites.len(), 1);
    assert_eq!(reloaded.saved.favorites[0].url, B);

    let mut restored = Rig::start(reloaded.saved);
    restored.until(|s| s.phase == Phase::Ready).await?;
    restored.settle().await;
    assert_eq!(restored.state().favorites.len(), 1);
    assert_eq!(restored.fake.started(), 0);
    assert_eq!(restored.fake.connects.load(SeqCst), 0);
    assert!(
        restored.fake.requests().is_empty(),
        "no prepare or playback"
    );

    restored
        .send(Command::SetFavorite {
            id: 1,
            url: B.into(),
            saved: false,
        })
        .await?;
    restored
        .until(|s| {
            s.favorite_result
                .as_ref()
                .is_some_and(|r| r.request_id == 1)
        })
        .await?;
    assert_eq!(restored.state().favorites, Vec::new());
    crate::store::write(&scratch.file("state.json"), &restored.saves.borrow())?;
    assert_eq!(
        crate::store::load(Some(scratch.file("state.json")))
            .saved
            .favorites,
        Vec::new()
    );
    Ok(())
}

#[tokio::test]
async fn stale_search_sessions_are_refused_and_change_nothing() -> Result<()> {
    let (mut controller, _fake, updates) = solo(Saved::default());
    controller.search.state = search_state(1, results(A));
    controller.save_search_favorite(10, 1, "aaaaaaaaaaa", true);
    let result = favorite_ack(&updates.borrow())?;
    assert_eq!(result.result, Ok(()));
    assert_eq!(result.url, A);
    assert_eq!(updates.borrow().favorites.len(), 1);

    controller.search.state = search_state(2, results(B));
    controller.save_search_favorite(11, 1, "aaaaaaaaaaa", true);
    let result = favorite_ack(&updates.borrow())?;
    assert_eq!(result.request_id, 11);
    assert_eq!(result.result, Err(FavoriteRefusal::StaleSearch));
    assert_eq!(
        updates.borrow().favorites.len(),
        1,
        "neither result is favorited"
    );

    controller.search.state = search_state(2, results(A));
    controller.save_search_favorite(12, 1, "aaaaaaaaaaa", true);
    let result = favorite_ack(&updates.borrow())?;
    assert_eq!(result.result, Err(FavoriteRefusal::StaleSearch));
    assert_eq!(
        updates.borrow().favorites.len(),
        1,
        "session validation is mandatory"
    );

    controller.save_search_favorite(13, 2, "aaaaaaaaaaa", true);
    let result = favorite_ack(&updates.borrow())?;
    assert_eq!(result.result, Ok(()));
    assert_eq!(updates.borrow().favorites.len(), 1);
    Ok(())
}

#[tokio::test]
async fn search_save_copies_result_facts_and_fills_missing_from_known() -> Result<()> {
    let mut saved = Saved::default();
    crate::history::record(
        &mut saved.history,
        HistoryEntry {
            url: A.into(),
            title: Some("From history".into()),
            duration_secs: None,
        },
    );
    let (mut controller, _fake, updates) = solo(saved);
    controller.search.state = SearchState {
        id: 1,
        query: "q".into(),
        phase: SearchPhase::Results(Results {
            items: vec![ResultItem {
                url: A.into(),
                title: None,
                channel: Some("Channel".into()),
                duration: Some(Duration::from_secs(45)),
                live: true,
            }],
            rejected: 0,
        }),
        status: None,
    };
    controller.save_search_favorite(1, 1, "aaaaaaaaaaa", true);
    let entry = &updates.borrow().favorites[0];
    assert_eq!(entry.url, A);
    assert_eq!(
        entry.title.as_deref(),
        Some("From history"),
        "filled from known facts"
    );
    assert_eq!(entry.channel.as_deref(), Some("Channel"));
    assert_eq!(entry.duration_secs, Some(45));
    assert!(entry.live);
    assert_eq!(favorite_ack(&updates.borrow())?.result, Ok(()));
    Ok(())
}

#[tokio::test]
async fn save_canonicalizes_aliases_and_remove_is_harmless_when_absent() -> Result<()> {
    let (mut controller, _fake, updates) = solo(Saved::default());
    controller.set_favorite(1, "https://youtu.be/aaaaaaaaaaa", true);
    assert_eq!(updates.borrow().favorites.len(), 1);
    assert_eq!(updates.borrow().favorites[0].url, A);

    controller.set_favorite(2, "https://www.youtube.com/watch?v=aaaaaaaaaaa&t=5", true);
    assert_eq!(
        updates.borrow().favorites.len(),
        1,
        "canonical identity deduplicates"
    );

    controller.set_favorite(3, B, false);
    assert_eq!(
        favorite_ack(&updates.borrow())?.result,
        Ok(()),
        "removing an absent entry succeeds"
    );
    assert_eq!(updates.borrow().favorites.len(), 1);
    let phase = updates.borrow().phase;
    controller.set_favorite(4, "not a video", true);
    assert_eq!(
        favorite_ack(&updates.borrow())?.result,
        Err(FavoriteRefusal::InvalidUrl)
    );
    assert_eq!(updates.borrow().phase, phase);
    controller.set_favorite(5, B, true);
    controller.set_favorite(6, A, false);
    controller.set_favorite(7, A, true);
    assert_eq!(updates.borrow().favorites[0].url, A);
    assert_eq!(updates.borrow().favorites[1].url, B);
    Ok(())
}

#[tokio::test]
async fn save_enriches_missing_facts_without_reordering_or_overwriting() -> Result<()> {
    let mut saved = Saved::default();
    crate::history::record(
        &mut saved.history,
        HistoryEntry {
            url: A.into(),
            title: Some("First title".into()),
            duration_secs: None,
        },
    );
    let (mut controller, _fake, updates) = solo(saved);
    controller.set_favorite(1, A, true);
    assert_eq!(
        updates.borrow().favorites[0].title.as_deref(),
        Some("First title")
    );
    assert!(updates.borrow().favorites[0].duration_secs.is_none());

    controller.set_favorite(2, B, true);
    assert_eq!(updates.borrow().favorites[0].url, B);
    assert_eq!(updates.borrow().favorites[1].url, A);

    crate::history::record(
        &mut controller.saved.history,
        HistoryEntry {
            url: A.into(),
            title: Some("Conflicting".into()),
            duration_secs: Some(90),
        },
    );
    controller.set_favorite(3, A, true);
    let known = &updates.borrow().favorites[1];
    assert_eq!(known.url, A, "position is preserved");
    assert_eq!(
        known.title.as_deref(),
        Some("First title"),
        "known fact is not overwritten"
    );
    assert_eq!(known.duration_secs, Some(90), "missing fact is filled");
    Ok(())
}

#[tokio::test]
async fn source_disappearance_still_saves_the_captured_url() -> Result<()> {
    let (mut controller, _fake, updates) = solo(Saved::default());
    controller.enqueue(A, Placement::End);
    controller.command(Command::ClearQueue).await;
    controller.set_favorite(1, A, true);
    assert_eq!(updates.borrow().favorites.len(), 1);
    assert_eq!(updates.borrow().favorites[0].url, A);
    assert!(
        updates.borrow().favorites[0].title.is_none(),
        "no fabricated metadata after the source disappears"
    );
    Ok(())
}

#[tokio::test]
async fn per_field_metadata_precedence_prefers_playback_then_history_then_queue() -> Result<()> {
    let (mut controller, fake, updates) = solo(Saved {
        remember_queue: true,
        queue: vec![SavedQueueItem {
            url: A.into(),
            title: Some("From queue".into()),
        }],
        history: vec![HistoryEntry {
            url: A.into(),
            title: Some("From history".into()),
            duration_secs: Some(100),
        }],
        ..Saved::default()
    });
    let track = track_of(&fake).await?;
    let mut current = active(&track);
    current.info = TrackInfo {
        title: Some("From playback".into()),
        duration: Some(Duration::from_secs(200)),
        seekable: true,
    };
    controller.track = Some(current);

    controller.set_favorite(1, A, true);
    let entry = updates.borrow().favorites[0].clone();
    assert_eq!(entry.title.as_deref(), Some("From playback"));
    assert_eq!(entry.duration_secs, Some(200));

    controller.saved.favorites.clear();
    controller
        .track
        .as_mut()
        .context("active playback")?
        .info
        .duration = None;
    controller.set_favorite(2, A, true);
    assert_eq!(
        updates.borrow().favorites[0].title.as_deref(),
        Some("From playback")
    );
    assert_eq!(updates.borrow().favorites[0].duration_secs, Some(100));

    controller.saved.favorites.clear();
    controller.search.state = search_state(
        1,
        Results {
            items: vec![ResultItem {
                title: Some("Selected title".into()),
                duration: Some(Duration::from_secs(45)),
                ..results(A).items.remove(0)
            }],
            rejected: 0,
        },
    );
    controller.save_search_favorite(3, 1, "aaaaaaaaaaa", true);
    assert_eq!(
        updates.borrow().favorites[0].title.as_deref(),
        Some("Selected title")
    );
    assert_eq!(updates.borrow().favorites[0].duration_secs, Some(45));

    controller.track = None;
    controller.saved.favorites.clear();
    controller.set_favorite(4, A, true);
    let entry = updates.borrow().favorites[0].clone();
    assert_eq!(entry.title.as_deref(), Some("From history"));
    assert_eq!(entry.duration_secs, Some(100));
    controller.saved.favorites.clear();
    controller.saved.history.clear();
    controller.set_favorite(5, A, true);
    assert_eq!(
        updates.borrow().favorites[0].title.as_deref(),
        Some("From queue")
    );
    Ok(())
}

#[tokio::test]
async fn membership_preserves_duplicate_queue_and_paused_seek_through_history_eviction()
-> Result<()> {
    let (mut controller, fake, updates) = solo(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    controller.enqueue(A, Placement::End);
    controller.enqueue(A, Placement::End);
    let track = track_of(&fake).await?;
    let calls = track.calls();
    let mut current = active(&track);
    current.paused = true;
    current.confirmed = true;
    current.position = Duration::from_secs(17);
    current.seek = SeekStatus::Complete(Duration::from_secs(17));
    controller.track = Some(current);
    let emitted = controller
        .track
        .as_ref()
        .context("active track")?
        .state(None);
    let track_id = emitted.track_id;
    controller.emit(emitted);
    controller.set_favorite(1, A, true);
    controller.set_favorite(2, A, true);
    for i in 0..=crate::history::HISTORY_LIMIT {
        crate::history::record(
            &mut controller.saved.history,
            HistoryEntry {
                url: format!("https://www.youtube.com/watch?v={i:011}"),
                title: None,
                duration_secs: None,
            },
        );
    }
    controller.sync();
    let state = updates.borrow().clone();
    assert_eq!(state.favorites.len(), 1);
    assert_eq!(state.favorites[0].url, A);
    assert!(state.history.iter().all(|entry| entry.url != A));
    assert_eq!(state.queue.len(), 2);
    assert!(state.queue.iter().all(|item| item.url == A));
    assert_eq!(state.preferences.repeat, RepeatMode::Queue);
    assert_eq!(state.phase, Phase::Paused);
    assert_eq!(state.track_id, track_id);
    assert_eq!(state.position, Duration::from_secs(17));
    assert_eq!(state.seek, SeekStatus::Complete(Duration::from_secs(17)));
    assert_eq!(track.calls(), calls);
    assert!(!track.stopped.load(SeqCst));
    Ok(())
}

#[tokio::test]
async fn capacity_refuses_new_members_but_accepts_existing() -> Result<()> {
    let saved = Saved {
        favorites: (0..FAVORITES_LIMIT)
            .map(|i| favorite(&format!("https://www.youtube.com/watch?v={i:0>11}"), None))
            .collect(),
        ..Saved::default()
    };
    let mut rig = Rig::start(saved);
    rig.until(|s| s.phase == Phase::Ready).await?;
    let existing = rig.state().favorites[0].url.clone();
    rig.send(Command::SetFavorite {
        id: 1,
        url: existing,
        saved: true,
    })
    .await?;
    let state = rig
        .until(|s| {
            s.favorite_result
                .as_ref()
                .is_some_and(|r| r.request_id == 1)
        })
        .await?;
    assert_eq!(favorite_ack(&state)?.result, Ok(()));
    assert_eq!(state.favorites.len(), FAVORITES_LIMIT);

    rig.send(Command::SetFavorite {
        id: 2,
        url: B.into(),
        saved: true,
    })
    .await?;
    let state = rig
        .until(|s| {
            s.favorite_result
                .as_ref()
                .is_some_and(|r| r.request_id == 2)
        })
        .await?;
    assert_eq!(favorite_ack(&state)?.result, Err(FavoriteRefusal::Full));
    assert_eq!(state.favorites.len(), FAVORITES_LIMIT);
    Ok(())
}

#[tokio::test]
async fn membership_works_while_discord_is_unavailable() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.health.send_modify(|health| {
        health.gateway = false;
        health.voice = false;
    });
    rig.until(|s| !s.available).await?;
    rig.send(Command::SetFavorite {
        id: 1,
        url: A.into(),
        saved: true,
    })
    .await?;
    let state = rig
        .until(|s| {
            s.favorite_result
                .as_ref()
                .is_some_and(|r| r.request_id == 1)
        })
        .await?;
    assert_eq!(state.favorites.len(), 1);
    assert_eq!(favorite_ack(&state)?.result, Ok(()));
    assert_eq!(rig.fake.started(), 0);
    assert_eq!(rig.fake.connects.load(SeqCst), 0);

    rig.send(Command::SetFavorite {
        id: 2,
        url: A.into(),
        saved: false,
    })
    .await?;
    rig.until(|s| {
        s.favorite_result
            .as_ref()
            .is_some_and(|r| r.request_id == 2)
    })
    .await?;
    assert_eq!(rig.state().favorites, Vec::new());
    Ok(())
}

#[tokio::test]
async fn favorites_survive_queue_changes_and_destination_switch() -> Result<()> {
    let seeded = favorite(A, Some("Kept"));
    let mut rig = Rig::start(Saved {
        favorites: vec![seeded.clone()],
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    rig.send(Command::ClearQueue).await?;
    rig.until(|s| s.queue.is_empty()).await?;
    rig.send(Command::Setup(crate::discord::setup::Request::Select(
        destination(4),
    )))
    .await?;
    let state = rig
        .until(|s| s.setup.destination == Some(destination(4)))
        .await?;
    assert_eq!(state.favorites, vec![seeded]);
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test]
async fn save_during_preparation_does_not_cancel_or_restart() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.fake.hang(A);
    rig.play(A).await?;
    rig.until(|s| s.phase == Phase::Resolving).await?;
    let requests = rig.fake.requests();
    rig.send(Command::SetFavorite {
        id: 1,
        url: B.into(),
        saved: true,
    })
    .await?;
    rig.until(|s| {
        s.favorite_result
            .as_ref()
            .is_some_and(|r| r.request_id == 1)
    })
    .await?;
    rig.settle().await;
    assert_eq!(
        rig.fake.requests(),
        requests,
        "no restart or new extraction"
    );
    assert_eq!(rig.fake.cancelled.load(SeqCst), 0);
    assert!(
        matches!(rig.state().phase, Phase::Resolving | Phase::Connecting),
        "preparation continues"
    );
    assert_eq!(rig.state().favorites.len(), 1);
    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    Ok(())
}

#[tokio::test]
async fn save_failure_is_reported_visibly_without_losing_membership() -> Result<()> {
    let scratch = Scratch::new()?;
    let path = scratch.file("state.json");
    crate::store::write(&path, &Saved::default())?;
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
        None,
        states,
        persist,
        Some(saver.status),
    );
    let task = tokio::spawn(controller.run(receiver, volume, stopping));

    commands
        .send(Command::SetFavorite {
            id: 1,
            url: A.into(),
            saved: true,
        })
        .await?;
    timeout(
        Duration::from_secs(5),
        updates.wait_for(|s| s.favorites.len() == 1),
    )
    .await??;

    let backup = scratch.file("previous.json");
    fs::rename(&path, &backup)?;
    fs::create_dir(&path)?;
    commands
        .send(Command::SetFavorite {
            id: 2,
            url: B.into(),
            saved: true,
        })
        .await?;
    let state = timeout(
        Duration::from_secs(5),
        updates.wait_for(|s| s.favorites.len() == 2),
    )
    .await??
    .clone();
    assert_eq!(favorite_ack(&state)?.result, Ok(()));
    let warned = timeout(
        Duration::from_secs(5),
        updates.wait_for(|s| {
            s.notice
                .as_deref()
                .is_some_and(|n| n.contains("Could not save"))
        }),
    )
    .await??
    .clone();
    assert_eq!(
        warned.favorites.len(),
        2,
        "in-memory membership survives a failed write"
    );

    fs::remove_dir(&path)?;
    fs::rename(&backup, &path)?;
    shutdown.send_replace(true);
    task.await?;
    saver.task.await?;
    Ok(())
}

#[tokio::test]
async fn local_changes_work_with_persistence_disabled() -> Result<()> {
    let loaded = crate::store::load(None);
    assert!(loaded.path.is_none(), "persistence is disabled");
    let fake = Fake::new();
    let (states, updates) = watch::channel(State::default());
    let (persist, _) = watch::channel(loaded.saved.clone());
    let mut controller = Controller::new(
        fake.clone(),
        loaded.saved,
        loaded.warning,
        states,
        persist,
        None,
    );
    controller.set_favorite(1, A, true);
    assert_eq!(updates.borrow().favorites.len(), 1);
    assert!(updates.borrow().favorite_result.is_some());
    assert_eq!(fake.requests(), Vec::<String>::new());
    assert_eq!(fake.started(), 0);
    assert_eq!(fake.connects.load(SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn current_url_tracks_active_and_preparing_requests() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    assert!(rig.state().current_url.is_none());

    rig.fake.tools_ready.send_replace(false);
    rig.play(A).await?;
    rig.until(|s| s.phase == Phase::Connecting).await?;
    assert_eq!(rig.state().current_url.as_deref(), Some(A));
    rig.fake.tools_ready.send_replace(true);
    rig.until(|s| s.phase == Phase::Resolving).await?;
    assert_eq!(rig.state().current_url.as_deref(), Some(A));

    rig.send(Command::Stop).await?;
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.play(B).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    assert_eq!(rig.state().current_url.as_deref(), Some(B));
    Ok(())
}

#[tokio::test]
async fn older_or_duplicate_request_ids_are_ignored() -> Result<()> {
    let (mut controller, _fake, updates) = solo(Saved::default());
    controller.set_favorite(1, A, true);
    controller.set_favorite(1, B, true);
    assert_eq!(updates.borrow().favorites.len(), 1);
    assert_eq!(updates.borrow().favorites[0].url, A);
    assert_eq!(favorite_ack(&updates.borrow())?.request_id, 1);

    controller.set_favorite(3, B, true);
    assert_eq!(updates.borrow().favorites.len(), 2);

    controller.set_favorite(2, A, false);
    assert_eq!(
        updates.borrow().favorites.len(),
        2,
        "an older id cannot undo a later request"
    );
    assert!(updates.borrow().favorites.iter().any(|f| f.url == A));
    assert_eq!(favorite_ack(&updates.borrow())?.request_id, 3);
    Ok(())
}

#[test]
fn snapshot_membership_lookup_uses_canonical_urls() {
    let state = State {
        favorites: vec![favorite(A, Some("Kept"))],
        ..State::default()
    };
    assert!(state.is_favorite(A));
    assert!(!state.is_favorite(B));
    assert!(!state.is_favorite("https://youtu.be/aaaaaaaaaaa"));
}

#[tokio::test]
async fn accepted_no_op_remove_persists_to_retry_an_earlier_failed_write() -> Result<()> {
    let fake = Fake::new();
    let (states, updates) = watch::channel(State::default());
    let (persist, mut saves) = watch::channel(Saved::default());
    let mut controller = Controller::new(fake, Saved::default(), None, states, persist, None);
    saves.borrow_and_update();
    controller.set_favorite(1, B, false);
    assert!(
        saves.has_changed().unwrap_or(false),
        "an accepted no-op Remove must persist so a later saver retries the write"
    );
    assert_eq!(updates.borrow().favorites, Vec::new());
    Ok(())
}

#[tokio::test]
async fn runtime_titles_and_channels_obey_safe_title_bounds() -> Result<()> {
    // Controller-owned playback facts are sanitized before storing.
    let (mut controller, fake, updates) = solo(Saved::default());
    let track = track_of(&fake).await?;
    let mut current = active(&track);
    let raw_title = format!("  Hello\u{1}\u{0}\nWorld\t{}  ", "ż".repeat(300));
    current.info = TrackInfo {
        title: Some(raw_title.clone()),
        duration: Some(Duration::from_secs(60)),
        seekable: true,
    };
    controller.track = Some(current);
    controller.set_favorite(1, A, true);
    let entry = updates.borrow().favorites[0].clone();
    assert_eq!(entry.title, crate::youtube::safe_title(&raw_title));
    assert!(
        entry
            .title
            .as_ref()
            .map_or(0, |title| title.chars().count())
            <= 200
    );

    // Search facts (title and channel) are sanitized too.
    controller.saved.favorites.clear();
    let raw_channel = format!("  Chan\u{2}\u{0}\nnel\t{}  ", "x".repeat(300));
    controller.search.state = search_state(
        1,
        Results {
            items: vec![ResultItem {
                url: A.into(),
                title: Some("  \u{3}Search\tTitle  ".into()),
                channel: Some(raw_channel.clone()),
                duration: Some(Duration::from_secs(30)),
                live: false,
            }],
            rejected: 0,
        },
    );
    controller.save_search_favorite(2, 1, "aaaaaaaaaaa", true);
    let entry = &updates.borrow().favorites[0];
    assert_eq!(
        entry.title.as_deref(),
        crate::youtube::safe_title("  \u{3}Search\tTitle  ").as_deref()
    );
    assert_eq!(entry.channel, crate::youtube::safe_title(&raw_channel));
    assert!(
        entry
            .channel
            .as_ref()
            .map_or(0, |channel| channel.chars().count())
            <= 200
    );
    Ok(())
}
