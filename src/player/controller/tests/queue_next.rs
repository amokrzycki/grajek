use super::*;
use crate::player::{Placement, RepeatMode};

const X: &str = "https://www.youtube.com/watch?v=xxxxxxxxxxx";
const Z: &str = "https://www.youtube.com/watch?v=zzzzzzzzzzz";

async fn queue_next(rig: &Rig, id: u64, url: &str) -> Result<()> {
    rig.send(Command::EnqueueRequest {
        id,
        value: url.into(),
        placement: Placement::Next,
    })
    .await
}

async fn move_to_next(rig: &Rig, request_id: u64, id: u64) -> Result<()> {
    rig.send(Command::MoveToNext { request_id, id }).await
}

async fn answered(rig: &mut Rig, request_id: u64) -> Result<State> {
    rig.until(|s| {
        s.enqueue_result
            .is_some_and(|result| result.request_id == request_id)
    })
    .await
}

async fn titled(rig: &mut Rig, count: usize) -> Result<State> {
    rig.until(|s| {
        s.queue.len() == count
            && s.queue
                .iter()
                .all(|item| item.title.is_some() || item.title_unavailable)
    })
    .await
}

fn ids(state: &State) -> Vec<u64> {
    state.queue.iter().map(|item| item.id).collect()
}

async fn playing_with_pending(rig: &mut Rig, paused: bool) -> Result<State> {
    rig.play(X).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    for url in [A, B, C] {
        rig.enqueue(url).await?;
    }
    if paused {
        rig.send(Command::Pause).await?;
        rig.until(|s| s.phase == Phase::Paused).await?;
    }
    titled(rig, 3).await
}

#[tokio::test]
async fn rotate_binds_its_successor_before_the_tool_check() -> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    for url in [A, B, C] {
        rig.enqueue(url).await?;
    }
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.tools_ready.send_replace(false);
    rig.fake.track(0)?.end();
    rig.until(|s| s.phase == Phase::Connecting).await?;
    queue_next(&rig, 1, X).await?;
    answered(&mut rig, 1).await?;
    assert_eq!(rig.queued(), [X, B, C]);
    rig.fake.tools_ready.send_replace(true);
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [A, B]);
    assert_eq!(rig.queued(), [X, C, A]);
    assert_eq!(rig.fake.started(), 2);
    Ok(())
}

#[tokio::test]
async fn rotate_with_nothing_pending_replays_the_completed_track() -> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.fake.tools_ready.send_replace(false);
    rig.fake.track(0)?.end();
    rig.until(|s| s.phase == Phase::Connecting).await?;
    queue_next(&rig, 1, X).await?;
    answered(&mut rig, 1).await?;
    rig.fake.tools_ready.send_replace(true);
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [A, A]);
    assert_eq!(rig.queued(), [X]);
    Ok(())
}

#[tokio::test]
async fn moving_leaves_the_active_track_alone_while_playing_or_paused() -> Result<()> {
    for paused in [false, true] {
        let mut rig = Rig::new().await?;
        let before = playing_with_pending(&mut rig, paused).await?;
        let [a, b, c] = ids(&before)[..] else {
            bail!("three pending entries expected");
        };
        let connects = rig.fake.connects.load(SeqCst);
        let disconnects = rig.fake.disconnects.load(SeqCst);
        move_to_next(&rig, 1, c).await?;
        let after = answered(&mut rig, 1).await?;
        assert_eq!(ids(&after), [c, a, b]);
        assert_eq!(after.queue[0], before.queue[2]);
        assert_eq!(after.queue[1], before.queue[0]);
        assert_eq!(after.queue[2], before.queue[1]);
        assert_eq!(after.track_id, before.track_id);
        assert_eq!(after.phase, before.phase);
        assert_eq!(after.enqueue_result.and_then(|r| r.item_id), Some(c));
        assert_eq!(rig.fake.started(), 1);
        assert_eq!(rig.fake.requests(), [X]);
        ensure!(!rig.fake.track(0)?.stopped.load(SeqCst));
        assert_eq!(rig.fake.connects.load(SeqCst), connects);
        assert_eq!(rig.fake.disconnects.load(SeqCst), disconnects);
    }
    Ok(())
}

#[tokio::test]
async fn queueing_next_leaves_the_active_track_alone_while_playing_or_paused() -> Result<()> {
    for paused in [false, true] {
        let mut rig = Rig::new().await?;
        let before = playing_with_pending(&mut rig, paused).await?;
        queue_next(&rig, 1, Z).await?;
        let after = answered(&mut rig, 1).await?;
        assert_eq!(rig.queued(), [Z, A, B, C]);
        assert_eq!(after.track_id, before.track_id);
        assert_eq!(after.phase, before.phase);
        assert_eq!(rig.fake.started(), 1);
        assert_eq!(rig.fake.requests(), [X]);
        ensure!(!rig.fake.track(0)?.stopped.load(SeqCst));
        let new = after.queue[0].id;
        assert!(!before.queue.iter().any(|item| item.id == new));
        assert_eq!(after.enqueue_result.and_then(|r| r.item_id), Some(new));
    }
    Ok(())
}

#[tokio::test]
async fn idle_queue_changes_never_play_or_join_voice() -> Result<()> {
    let mut rig = Rig::new().await?;
    for url in [A, B] {
        rig.enqueue(url).await?;
    }
    let state = rig.until(|s| s.queue.len() == 2).await?;
    queue_next(&rig, 1, X).await?;
    answered(&mut rig, 1).await?;
    assert_eq!(rig.queued(), [X, A, B]);
    move_to_next(&rig, 2, state.queue[1].id).await?;
    answered(&mut rig, 2).await?;
    assert_eq!(rig.queued(), [B, X, A]);
    rig.settle().await;
    assert_eq!(rig.state().phase, Phase::Ready);
    assert_eq!(rig.fake.started(), 0);
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    assert_eq!(rig.fake.requests(), Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn the_entry_being_prepared_still_plays_and_the_new_one_is_first_afterwards() -> Result<()> {
    for moved in [false, true] {
        let mut rig = Rig::new().await?;
        for url in [A, B] {
            rig.enqueue(url).await?;
        }
        let pending = rig.until(|s| s.queue.len() == 2).await?;
        rig.fake.tools_ready.send_replace(false);
        rig.send(Command::Next).await?;
        rig.until(|s| s.phase == Phase::Connecting).await?;
        if moved {
            move_to_next(&rig, 1, pending.queue[1].id).await?;
        } else {
            queue_next(&rig, 1, X).await?;
        }
        answered(&mut rig, 1).await?;
        assert_eq!(rig.queued().len(), if moved { 2 } else { 3 });
        assert_eq!(rig.fake.started(), 0);
        rig.fake.tools_ready.send_replace(true);
        rig.started(1).await?;
        assert_eq!(rig.fake.requests(), [A]);
        let first = if moved { B } else { X };
        assert_eq!(rig.queued().first().map(String::as_str), Some(first));
        assert!(!rig.queued().iter().any(|url| url == A));
    }
    Ok(())
}

#[tokio::test]
async fn natural_end_and_next_consume_a_moved_entry_exactly_once() -> Result<()> {
    for natural in [true, false] {
        let mut rig = Rig::new().await?;
        rig.play(A).await?;
        rig.started(1).await?;
        rig.fake.track(0)?.advance(1);
        rig.until(|s| s.phase == Phase::Playing).await?;
        rig.enqueue(B).await?;
        rig.enqueue(C).await?;
        let state = rig.until(|s| s.queue.len() == 2).await?;
        move_to_next(&rig, 1, state.queue[1].id).await?;
        answered(&mut rig, 1).await?;
        assert_eq!(rig.queued(), [C, B]);
        if natural {
            rig.fake.track(0)?.end();
        } else {
            rig.send(Command::Next).await?;
        }
        rig.started(2).await?;
        rig.settle().await;
        assert_eq!(rig.fake.track(1)?.title, C);
        assert_eq!(rig.fake.requests(), [A, C]);
        assert_eq!(rig.queued(), [B]);
        assert_eq!(rig.fake.started(), 2);
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_track_keeps_the_current_track_and_next_plays_the_moved_entry() -> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Track,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.play(X).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    rig.enqueue(A).await?;
    rig.enqueue(B).await?;
    let state = rig.until(|s| s.queue.len() == 2).await?;
    move_to_next(&rig, 1, state.queue[1].id).await?;
    answered(&mut rig, 1).await?;
    rig.fake.track(0)?.end();
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [X, X]);
    assert_eq!(rig.queued(), [B, A]);
    rig.send(Command::Next).await?;
    rig.started(3).await?;
    assert_eq!(rig.fake.requests(), [X, X, B]);
    assert_eq!(rig.queued(), [A]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn repeat_queue_plays_the_moved_entry_and_rotates_the_completed_track_to_the_back()
-> Result<()> {
    let mut rig = Rig::start(Saved {
        repeat: RepeatMode::Queue,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    for url in [A, B, C] {
        rig.enqueue(url).await?;
    }
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(1);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let state = rig.until(|s| s.queue.len() == 2).await?;
    move_to_next(&rig, 1, state.queue[1].id).await?;
    answered(&mut rig, 1).await?;
    rig.fake.track(0)?.end();
    rig.started(2).await?;
    assert_eq!(rig.fake.requests(), [A, C]);
    assert_eq!(rig.queued(), [B, A]);
    Ok(())
}

#[tokio::test]
async fn boundaries_are_silent_or_refused_without_changing_the_queue() -> Result<()> {
    let mut rig = Rig::new().await?;
    for url in [A, B, C] {
        rig.enqueue(url).await?;
    }
    let before = titled(&mut rig, 3).await?;
    move_to_next(&rig, 1, 999).await?;
    let stale = answered(&mut rig, 1).await?;
    assert_eq!(stale.enqueue_result.and_then(|r| r.item_id), None);
    assert_eq!(stale.queue, before.queue);
    assert_eq!(stale.error, None);
    move_to_next(&rig, 2, before.queue[0].id).await?;
    let first = answered(&mut rig, 2).await?;
    assert_eq!(
        first.enqueue_result.and_then(|r| r.item_id),
        Some(before.queue[0].id)
    );
    assert_eq!(first.queue, before.queue);
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    move_to_next(&rig, 3, before.queue[0].id).await?;
    let consumed = answered(&mut rig, 3).await?;
    assert_eq!(consumed.enqueue_result.and_then(|r| r.item_id), None);
    assert_eq!(consumed.error, None);
    assert_eq!(rig.queued(), [B, C]);
    Ok(())
}

#[tokio::test]
async fn a_full_queue_refuses_queue_next_but_still_moves() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        queue: (0..QUEUE_LIMIT)
            .map(|index| SavedQueueItem {
                url: format!("https://www.youtube.com/watch?v={index:011}"),
                title: Some(format!("Track {index}")),
            })
            .collect(),
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    let before = rig.state();
    queue_next(&rig, 1, X).await?;
    let refused = answered(&mut rig, 1).await?;
    assert_eq!(refused.enqueue_result.and_then(|r| r.item_id), None);
    assert_eq!(refused.queue, before.queue);
    assert!(refused.error.is_some(), "the refusal is reported");
    let last = before.queue[QUEUE_LIMIT - 1].id;
    move_to_next(&rig, 2, last).await?;
    let moved = answered(&mut rig, 2).await?;
    assert_eq!(moved.enqueue_result.and_then(|r| r.item_id), Some(last));
    assert_eq!(moved.queue.len(), QUEUE_LIMIT);
    assert_eq!(moved.queue[0], before.queue[QUEUE_LIMIT - 1]);
    assert_eq!(moved.queue[1], before.queue[0]);
    Ok(())
}

#[tokio::test]
async fn a_late_title_reply_fills_the_entry_at_its_new_position() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        queue: vec![
            SavedQueueItem {
                url: A.into(),
                title: Some("A".into()),
            },
            SavedQueueItem {
                url: B.into(),
                title: Some("B".into()),
            },
            SavedQueueItem {
                url: C.into(),
                title: None,
            },
        ],
        ..Saved::default()
    });
    let reply = rig.fake.title_reply(C);
    rig.until(|s| s.phase == Phase::Ready).await?;
    eventually(|| rig.fake.title_requests() == [C]).await?;
    let c = rig.state().queue[2].id;
    move_to_next(&rig, 1, c).await?;
    answered(&mut rig, 1).await?;
    assert_eq!(rig.state().queue[0].label(), "Loading title…");
    reply
        .send(Ok(Some("Late C".into())))
        .map_err(|_| anyhow::anyhow!("metadata cancelled"))?;
    let state = rig
        .until(|s| s.queue.first().and_then(|item| item.title.as_deref()) == Some("Late C"))
        .await?;
    assert_eq!(state.queue[0].id, c);
    assert_eq!(state.queue[1].title.as_deref(), Some("A"));
    assert_eq!(state.queue[2].title.as_deref(), Some("B"));
    Ok(())
}

#[tokio::test]
async fn queue_next_reuses_a_title_known_from_history() -> Result<()> {
    let mut saved = Saved::default();
    saved.record(crate::store::HistoryEntry {
        url: X.into(),
        title: Some("Known".into()),
        duration_secs: None,
    });
    let mut rig = Rig::start(saved);
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    queue_next(&rig, 1, X).await?;
    let state = answered(&mut rig, 1).await?;
    assert_eq!(state.queue[0].title.as_deref(), Some("Known"));
    rig.settle().await;
    assert!(!rig.fake.title_requests().iter().any(|url| url == X));
    Ok(())
}

#[tokio::test]
async fn queue_next_of_a_pending_url_adds_a_distinct_second_entry() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.enqueue(A).await?;
    let before = rig.until(|s| s.queue.len() == 1).await?;
    queue_next(&rig, 1, A).await?;
    let state = answered(&mut rig, 1).await?;
    assert_eq!(rig.queued(), [A, A]);
    assert_ne!(state.queue[0].id, before.queue[0].id);
    assert_eq!(state.queue[1].id, before.queue[0].id);
    Ok(())
}
