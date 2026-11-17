use super::*;
use crate::player::{SearchAction, SearchPhase};
use crate::search::{Error, ResultItem, Results};

pub(super) struct Call {
    query: String,
    reply: oneshot::Sender<std::result::Result<Results, Error>>,
    cancel: watch::Receiver<bool>,
    cleanup: oneshot::Sender<()>,
}

fn listen(fake: &Fake) -> mpsc::UnboundedReceiver<Call> {
    let (sender, receiver) = mpsc::unbounded_channel();
    *fake
        .search_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(sender);
    receiver
}

fn results(url: &str) -> Results {
    Results {
        items: vec![ResultItem {
            url: url.into(),
            title: Some("Factual title".into()),
            channel: None,
            duration: None,
            live: false,
        }],
        rejected: 0,
    }
}

async fn submit(rig: &mut Rig, id: u64, query: &str) -> Result<()> {
    rig.send(Command::Search {
        id,
        query: query.into(),
    })
    .await?;
    rig.until(|s| s.search.id == id && s.search.phase == SearchPhase::Loading)
        .await?;
    Ok(())
}

async fn complete(rig: &mut Rig, call: Call, id: u64, url: &str) -> Result<()> {
    assert!(!*call.cancel.borrow());
    call.reply
        .send(Ok(results(url)))
        .map_err(|_| anyhow::anyhow!("search worker disappeared"))?;
    rig.until(|s| s.search.id == id && matches!(s.search.phase, SearchPhase::Results(_)))
        .await?;
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::print_stderr,
    reason = "The bounded responsiveness regression retains timing evidence"
)]
async fn replacement_rejects_late_completion_and_keeps_transport_responsive() -> Result<()> {
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        queue: vec![SavedQueueItem {
            url: B.into(),
            title: Some("Queued".into()),
        }],
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.play(A).await?;
    rig.started(1).await?;
    let track = rig.fake.track(0)?;
    track.advance(2);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let track_id = rig.state().track_id;
    let mut calls = listen(&rig.fake);
    let saved = rig.saves.borrow_and_update().clone();
    submit(&mut rig, 1, "Q1").await?;
    let mut old = calls.recv().await.context("Q1 missing")?;
    submit(&mut rig, 2, "Q2").await?;
    old.cancel.wait_for(|cancel| *cancel).await?;
    for action in [
        SearchAction::Play,
        SearchAction::Enqueue {
            id: 1,
            placement: crate::player::Placement::End,
        },
    ] {
        rig.send(Command::SearchAction {
            search_id: 1,
            video_id: "aaaaaaaaaaa".into(),
            action,
        })
        .await?;
    }
    let transport_started = std::time::Instant::now();
    timeout(Duration::from_millis(250), async {
        rig.send(Command::Transport {
            id: 11,
            command: Box::new(Command::Pause),
        })
        .await?;
        rig.until(|s| s.transport_ack == 11 && s.phase == Phase::Paused)
            .await?;
        rig.send(Command::Transport {
            id: 12,
            command: Box::new(Command::Resume),
        })
        .await?;
        rig.until(|s| s.transport_ack == 12 && s.phase == Phase::Playing)
            .await?;
        rig.set_volume(37, false);
        rig.until(|s| s.volume_seq == 1 && s.volume == 37).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("transport blocked by search cleanup")??;
    eprintln!(
        "SEARCH_TRANSPORT pause_resume_volume_ack_us={}",
        transport_started.elapsed().as_micros()
    );
    assert!(track.calls().ends_with(&["pause", "resume", "volume"]));
    assert_eq!(rig.state().track_id, track_id);
    assert_eq!(*rig.saves.borrow(), saved);
    assert!(!rig.saves.has_changed()?);
    assert!(calls.try_recv().is_err(), "replacement waits for cleanup");
    old.cleanup
        .send(())
        .map_err(|()| anyhow::anyhow!("cleanup worker disappeared"))?;
    let next = timeout(Duration::from_secs(1), calls.recv())
        .await?
        .context("Q2 missing")?;
    assert_eq!(next.query, "Q2");
    complete(&mut rig, next, 2, C).await?;
    assert_eq!(rig.fake.started(), 1);
    assert_eq!(rig.fake.requests(), vec![A]);
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    assert_eq!(rig.queued(), vec![B]);
    Ok(())
}

#[tokio::test]
async fn stale_completion_reducer_cannot_publish_or_mutate() -> Result<()> {
    let fake = Fake::new();
    let (states, state) = watch::channel(State::default());
    let (persist, mut saved) = watch::channel(Saved::default());
    let mut controller = Controller::new(fake, Saved::default(), None, states, persist, None);
    controller.submit_search(2, "current".into());
    saved.borrow_and_update();
    controller.apply_search_result(1, Ok(results(A)));
    assert_eq!(state.borrow().search.id, 2);
    assert_eq!(state.borrow().search.phase, SearchPhase::Loading);
    assert!(!saved.has_changed()?);
    controller.cancel_search(2);
    controller.apply_search_result(2, Ok(results(A)));
    assert_eq!(state.borrow().search.phase, SearchPhase::Idle);
    controller.join_search().await;
    Ok(())
}

#[tokio::test]
async fn selected_actions_recheck_capacity_availability_and_consume_once() -> Result<()> {
    for remember in [false, true] {
        let mut rig = Rig::start(Saved {
            remember_queue: remember,
            ..Saved::default()
        });
        rig.until(|s| s.phase == Phase::Ready).await?;
        let mut calls = listen(&rig.fake);
        submit(&mut rig, 1, "music").await?;
        complete(
            &mut rig,
            calls.recv().await.context("search call missing")?,
            1,
            A,
        )
        .await?;
        rig.fake.health.send_modify(|h| h.gateway = false);
        rig.until(|s| !s.available).await?;
        for action in [
            SearchAction::Play,
            SearchAction::Enqueue {
                id: 1,
                placement: crate::player::Placement::End,
            },
        ] {
            rig.send(Command::SearchAction {
                search_id: 1,
                video_id: "aaaaaaaaaaa".into(),
                action,
            })
            .await?;
        }
        rig.until(|s| s.search.status.is_some()).await?;
        assert!(matches!(rig.state().search.phase, SearchPhase::Results(_)));
        assert_eq!(rig.queued(), Vec::<String>::new());
        rig.fake.health.send_modify(|h| h.gateway = true);
        rig.until(|s| s.available).await?;
        for _ in 0..100 {
            rig.enqueue(B).await?;
        }
        rig.until(|s| s.queue.len() == 100).await?;
        rig.send(Command::SearchAction {
            search_id: 1,
            video_id: "aaaaaaaaaaa".into(),
            action: SearchAction::Enqueue {
                id: 2,
                placement: crate::player::Placement::End,
            },
        })
        .await?;
        rig.until(|s| s.enqueue_result.is_some_and(|r| r.request_id == 2))
            .await?;
        assert!(matches!(rig.state().search.phase, SearchPhase::Results(_)));
        let remove = rig.state().queue[0].id;
        rig.send(Command::Dequeue(remove)).await?;
        for id in [3, 4] {
            rig.send(Command::SearchAction {
                search_id: 1,
                video_id: "aaaaaaaaaaa".into(),
                action: SearchAction::Enqueue {
                    id,
                    placement: crate::player::Placement::End,
                },
            })
            .await?;
        }
        rig.until(|s| s.search.phase == SearchPhase::Idle).await?;
        assert_eq!(
            rig.queued().iter().filter(|url| url.as_str() == A).count(),
            1
        );
        assert_eq!(rig.fake.started(), 0);
    }
    Ok(())
}

pub(super) async fn run(
    fake: &Fake,
    query: String,
    mut cancel: watch::Receiver<bool>,
) -> std::result::Result<Results, Error> {
    let sender = fake
        .search_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or(Error::Unavailable)?;
    let (reply, response) = oneshot::channel();
    let (cleanup, cleaned) = oneshot::channel();
    sender
        .send(Call {
            query,
            reply,
            cancel: cancel.clone(),
            cleanup,
        })
        .map_err(|_| Error::Unavailable)?;
    tokio::select! {
        result = response => result.unwrap_or(Err(Error::Unavailable)),
        () = async { let _ = cancel.wait_for(|cancel| *cancel).await; } => {
            let _ = cleaned.await;
            Err(Error::Cancelled)
        }
    }
}

#[tokio::test]
async fn rapid_replacement_and_shutdown_keep_one_owned_worker() -> Result<()> {
    let mut rig = Rig::new().await?;
    let mut calls = listen(&rig.fake);
    submit(&mut rig, 1, "Q1").await?;
    let old = calls.recv().await.context("Q1 missing")?;
    submit(&mut rig, 2, "Q2").await?;
    submit(&mut rig, 3, "Q3").await?;
    assert!(calls.try_recv().is_err());
    old.cleanup
        .send(())
        .map_err(|()| anyhow::anyhow!("cleanup worker disappeared"))?;
    let mut current = timeout(Duration::from_secs(1), calls.recv())
        .await?
        .context("Q3 missing")?;
    assert_eq!(current.query, "Q3");
    rig.shutdown.send_replace(true);
    current.cancel.wait_for(|cancel| *cancel).await?;
    assert!(!rig.task.is_finished(), "shutdown must join cleanup");
    current
        .cleanup
        .send(())
        .map_err(|()| anyhow::anyhow!("cleanup worker disappeared"))?;
    timeout(Duration::from_secs(1), rig.task).await??;
    assert!(rig.fake.shut_down.load(SeqCst));
    Ok(())
}

#[tokio::test]
async fn forged_and_replayed_actions_are_inert_and_play_consumes_once() -> Result<()> {
    let mut rig = Rig::new().await?;
    let mut calls = listen(&rig.fake);
    submit(&mut rig, 5, "music").await?;
    complete(
        &mut rig,
        calls.recv().await.context("search call missing")?,
        5,
        A,
    )
    .await?;
    rig.saves.borrow_and_update();
    rig.send(Command::SearchAction {
        search_id: 5,
        video_id: "bbbbbbbbbbb".into(),
        action: SearchAction::Play,
    })
    .await?;
    rig.send(Command::Search {
        id: 4,
        query: "old".into(),
    })
    .await?;
    rig.send(Command::CancelSearch { id: 4 }).await?;
    rig.send(Command::Transport {
        id: 8,
        command: Box::new(Command::SearchAction {
            search_id: 5,
            video_id: "aaaaaaaaaaa".into(),
            action: SearchAction::Play,
        }),
    })
    .await?;
    rig.until(|s| s.transport_ack == 8).await?;
    rig.started(1).await?;
    rig.send(Command::SearchAction {
        search_id: 5,
        video_id: "aaaaaaaaaaa".into(),
        action: SearchAction::Play,
    })
    .await?;
    rig.send(Command::Search {
        id: 5,
        query: "replay".into(),
    })
    .await?;
    rig.until(|s| s.search.phase == SearchPhase::Idle).await?;
    assert_eq!(rig.fake.requests(), [A]);
    assert!(calls.try_recv().is_err());
    assert!(!rig.saves.has_changed()?);
    Ok(())
}

#[tokio::test]
async fn search_failures_and_empty_results_are_transient_without_components_navigation()
-> Result<()> {
    let mut rig = Rig::new().await?;
    let mut calls = listen(&rig.fake);
    let baseline = rig.saves.borrow_and_update().clone();
    for (index, error) in [
        Error::MissingComponents,
        Error::Unavailable,
        Error::Timeout,
        Error::Malformed,
    ]
    .into_iter()
    .enumerate()
    {
        let id = index as u64 + 1;
        submit(&mut rig, id, "music").await?;
        calls
            .recv()
            .await
            .context("search call missing")?
            .reply
            .send(Err(error))
            .map_err(|_| anyhow::anyhow!("search worker disappeared"))?;
        let state = rig
            .until(|s| matches!(s.search.phase, SearchPhase::Failed(_)))
            .await?;
        assert_eq!(state.components_request, 0);
        assert_eq!(state.phase, Phase::Ready);
        assert!(state.error.is_none());
    }
    submit(&mut rig, 10, "empty").await?;
    calls
        .recv()
        .await
        .context("search call missing")?
        .reply
        .send(Ok(Results {
            items: Vec::new(),
            rejected: 3,
        }))
        .map_err(|_| anyhow::anyhow!("search worker disappeared"))?;
    rig.until(|s| s.search.phase == SearchPhase::Empty { rejected: 3 })
        .await?;
    rig.send(Command::CancelSearch { id: 10 }).await?;
    rig.until(|s| s.search.phase == SearchPhase::Idle).await?;
    assert_eq!(*rig.saves.borrow(), baseline);
    assert!(!rig.saves.has_changed()?);
    assert_eq!(rig.fake.connects.load(SeqCst), 0);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn cleanup_watchdog_retains_worker_and_suppresses_replacement_until_reaped() -> Result<()> {
    let fake = Fake::new();
    let mut calls = listen(&fake);
    let (states, _) = watch::channel(State::default());
    let (persist, _) = watch::channel(Saved::default());
    let mut controller = Controller::new(fake, Saved::default(), None, states, persist, None);
    controller.submit_search(1, "first".into());
    let old = calls.recv().await.context("first missing")?;
    controller.submit_search(2, "next".into());
    tokio::time::advance(Duration::from_secs(2)).await;
    controller.search_expired();
    assert!(controller.search.state.status.is_some());
    assert!(controller.search.job.is_some());
    controller.submit_search(3, "must wait".into());
    assert_eq!(controller.search.state.query, "must wait");
    assert!(
        matches!(controller.search.state.phase, SearchPhase::Failed(_)),
        "stuck cleanup blocks new searches"
    );
    assert!(calls.try_recv().is_err());
    old.cleanup
        .send(())
        .map_err(|()| anyhow::anyhow!("cleanup worker disappeared"))?;
    let result = super::super::search::result(&mut controller.search.job).await;
    controller.finish_search(result);
    assert!(calls.try_recv().is_err());
    controller.submit_search(4, "after cleanup".into());
    let call = calls.recv().await.context("new search missing")?;
    call.reply
        .send(Ok(results(A)))
        .map_err(|_| anyhow::anyhow!("search worker disappeared"))?;
    let result = super::super::search::result(&mut controller.search.job).await;
    controller.finish_search(result);
    assert!(matches!(
        controller.search.state.phase,
        SearchPhase::Results(_)
    ));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn completion_at_the_original_deadline_is_rejected_before_the_timer_branch() -> Result<()> {
    let fake = Fake::new();
    let mut calls = listen(&fake);
    let (states, _) = watch::channel(State::default());
    let (persist, _) = watch::channel(Saved::default());
    let mut controller = Controller::new(fake, Saved::default(), None, states, persist, None);
    controller.submit_search(1, "bounded".into());
    let call = calls.recv().await.context("search call missing")?;
    tokio::time::advance(crate::search::TIMEOUT).await;
    call.reply
        .send(Ok(results(A)))
        .map_err(|_| anyhow::anyhow!("search worker disappeared"))?;
    let result = super::super::search::result(&mut controller.search.job).await;
    controller.finish_search(result);
    assert_eq!(
        controller.search.state.phase,
        SearchPhase::Failed(Error::Timeout)
    );
    Ok(())
}

async fn queue_next_from(rig: &Rig, search_id: u64, video_id: &str, id: u64) -> Result<()> {
    rig.send(Command::SearchAction {
        search_id,
        video_id: video_id.into(),
        action: SearchAction::Enqueue {
            id,
            placement: crate::player::Placement::Next,
        },
    })
    .await
}

fn unanswered(rig: &Rig, id: u64) -> bool {
    rig.state()
        .enqueue_result
        .is_none_or(|result| result.request_id != id)
}

#[tokio::test]
async fn stale_or_unknown_search_selections_cannot_queue_next() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    let mut calls = listen(&rig.fake);
    submit(&mut rig, 1, "music").await?;
    complete(
        &mut rig,
        calls.recv().await.context("search call missing")?,
        1,
        A,
    )
    .await?;
    queue_next_from(&rig, 1, "bbbbbbbbbbb", 5).await?;
    rig.send(Command::Transport {
        id: 1,
        command: Box::new(Command::Pause),
    })
    .await?;
    rig.until(|s| s.transport_ack == 1).await?;
    assert_eq!(rig.queued(), [B], "a video outside the results is inert");
    assert!(unanswered(&rig, 5));
    assert!(matches!(rig.state().search.phase, SearchPhase::Results(_)));

    submit(&mut rig, 2, "other").await?;
    queue_next_from(&rig, 1, "aaaaaaaaaaa", 6).await?;
    rig.send(Command::Transport {
        id: 2,
        command: Box::new(Command::Pause),
    })
    .await?;
    rig.until(|s| s.transport_ack == 2).await?;
    assert_eq!(rig.queued(), [B], "a superseded session cannot act");
    assert!(unanswered(&rig, 6));

    rig.send(Command::CancelSearch { id: 2 }).await?;
    rig.until(|s| s.search.phase == SearchPhase::Idle).await?;
    queue_next_from(&rig, 2, "aaaaaaaaaaa", 7).await?;
    rig.send(Command::Transport {
        id: 3,
        command: Box::new(Command::Pause),
    })
    .await?;
    rig.until(|s| s.transport_ack == 3).await?;
    assert_eq!(rig.queued(), [B], "a cancelled session cannot act");
    assert!(unanswered(&rig, 7));
    Ok(())
}

#[tokio::test]
async fn a_live_search_session_queues_next_exactly_once() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    let mut calls = listen(&rig.fake);
    submit(&mut rig, 1, "music").await?;
    complete(
        &mut rig,
        calls.recv().await.context("search call missing")?,
        1,
        A,
    )
    .await?;
    for id in [3, 4] {
        queue_next_from(&rig, 1, "aaaaaaaaaaa", id).await?;
    }
    rig.until(|s| s.enqueue_result.is_some_and(|r| r.request_id == 3))
        .await?;
    rig.send(Command::Transport {
        id: 1,
        command: Box::new(Command::Pause),
    })
    .await?;
    rig.until(|s| s.transport_ack == 1).await?;
    assert_eq!(rig.queued(), [A, B]);
    assert!(unanswered(&rig, 4));
    assert_eq!(rig.state().search.phase, SearchPhase::Idle);
    assert_eq!(rig.fake.started(), 0);
    Ok(())
}
