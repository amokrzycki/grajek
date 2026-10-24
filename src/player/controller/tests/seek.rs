use super::*;

impl FakeTrack {
    fn seek_gate(&self) -> oneshot::Sender<TrackResult<Duration>> {
        let (tx, rx) = oneshot::channel();
        *self
            .seek_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(rx);
        tx
    }
    fn seeks(&self) -> Vec<Duration> {
        self.seeks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn playing_rig() -> Result<Rig> {
    let mut rig = Rig::new().await?;
    rig.play(A).await?;
    rig.started(1).await?;
    rig.fake.track(0)?.advance(10);
    rig.until(|state| state.phase == Phase::Playing).await?;
    Ok(rig)
}

async fn request(rig: &Rig, seconds: u64) -> Result<()> {
    rig.send(Command::Seek {
        track_id: rig.state().track_id,
        position: Duration::from_secs(seconds),
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn seeks_forward_backward_and_paused_use_actual_results_without_new_history() -> Result<()> {
    let mut rig = playing_rig().await?;
    rig.set_volume(40, true);
    rig.until(|s| s.volume == 40).await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    let track = rig.fake.track(0)?;
    let id = rig.state().track_id;
    for (target, actual, paused) in [
        (40, 39, false),
        (20, 19, false),
        (30, 29, true),
        (15, 14, false),
    ] {
        rig.send(if paused {
            Command::Pause
        } else {
            Command::Resume
        })
        .await?;
        rig.until(|s| {
            s.phase
                == if paused {
                    Phase::Paused
                } else {
                    Phase::Playing
                }
        })
        .await?;
        *track.mode.lock().unwrap_or_else(PoisonError::into_inner) = (
            if paused {
                PlayMode::Pause
            } else {
                PlayMode::Play
            },
            rig.state().position,
        );
        let gate = track.seek_gate();
        let before = rig.state().position;
        request(&rig, target).await?;
        rig.until(|s| matches!(s.seek, SeekStatus::Pending(_)))
            .await?;
        assert_eq!(
            rig.state().position,
            before,
            "pending seek is not a position"
        );
        gate.send(Ok(Duration::from_secs(actual)))
            .map_err(|_| anyhow::anyhow!("seek cancelled"))?;
        rig.until(|s| s.seek == SeekStatus::Complete(Duration::from_secs(actual)))
            .await?;
        assert_eq!(rig.state().position.as_secs(), actual);
        assert_eq!(rig.state().track_id, id);
        assert_eq!(
            rig.state().phase,
            if paused {
                Phase::Paused
            } else {
                Phase::Playing
            }
        );
        assert_eq!(rig.state().history.len(), 1);
        assert_eq!(rig.queued(), [B]);
        assert!((track.volume() - 0.4).abs() < f32::EPSILON);
        assert_eq!(rig.fake.started(), 1);
    }
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn rapid_seeks_coalesce_to_last_target_and_duplicates_do_not_repeat() -> Result<()> {
    let mut rig = playing_rig().await?;
    let track = rig.fake.track(0)?;
    let gate = track.seek_gate();
    for second in [20, 20, 30, 40, 40] {
        request(&rig, second).await?;
    }
    rig.until(|s| s.seek == SeekStatus::Pending(Duration::from_secs(40)))
        .await?;
    eventually(|| track.seeks().len() == 1).await?;
    gate.send(Ok(Duration::from_secs(20)))
        .map_err(|_| anyhow::anyhow!("seek cancelled"))?;
    rig.until(|s| s.seek == SeekStatus::Complete(Duration::from_secs(40)))
        .await?;
    assert_eq!(
        track.seeks(),
        [Duration::from_secs(20), Duration::from_secs(40)]
    );
    request(&rig, 40).await?;
    rig.settle().await;
    assert_eq!(track.seeks().len(), 2);
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn seek_failure_keeps_position_queue_and_failure_budget() -> Result<()> {
    let (mut controller, fake, updates) = solo(Saved {
        continue_on_error: true,
        ..Saved::default()
    });
    let track = track_of(&fake).await?;
    let mut current = active(&track);
    current.confirmed = true;
    current.position = Duration::from_secs(10);
    controller.track = Some(current);
    controller.failures = 2;
    controller.queued_track = true;
    controller.enqueue(B);
    controller.emit(controller.track.as_ref().context("no track")?.state(None));
    let gate = track.seek_gate();
    controller.seek(0, Duration::from_secs(30));
    gate.send(Err(ControlError::Finished))
        .map_err(|_| anyhow::anyhow!("seek cancelled"))?;
    let result = seek_result(&mut controller.seek_job).await;
    controller.finish_seek(result);
    assert!(matches!(updates.borrow().seek, SeekStatus::Failed(_)));
    assert_eq!(updates.borrow().position.as_secs(), 10);
    assert_eq!(controller.failures, 2);
    assert!(controller.scheduled.is_none());
    assert_eq!(controller.queue.snapshot().len(), 1);
    assert!(controller.track.is_some());
    assert!(!updates.borrow().can_seek());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn seek_is_rejected_in_idle_resolving_stopped_and_unreliable_media() -> Result<()> {
    let (mut controller, fake, updates) = solo(Saved::default());
    controller.publish(Phase::Ready, None);
    controller.seek(0, Duration::ZERO);
    assert!(controller.seek_job.is_none());
    let track = track_of(&fake).await?;
    controller.track = Some(active(&track));
    for (phase, duration, seekable) in [
        (Phase::Resolving, Some(Duration::from_secs(60)), true),
        (Phase::Playing, None, true),
        (Phase::Playing, Some(Duration::ZERO), true),
        (Phase::Playing, Some(Duration::from_secs(60)), false),
    ] {
        let current = controller.track.as_mut().context("no track")?;
        current.info.duration = duration;
        current.info.seekable = seekable;
        controller.publish(phase, None);
        let id = updates.borrow().track_id;
        controller.seek(id, Duration::from_secs(20));
        assert!(controller.seek_job.is_none());
    }
    controller.stop().await;
    controller.seek(0, Duration::from_secs(20));
    assert_eq!(track.seeks(), [] as [Duration; 0]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn every_track_replacement_cancels_pending_seek_and_stale_intents() -> Result<()> {
    for action in [
        "next",
        "stop",
        "play",
        "leave",
        "end",
        "disconnect",
        "shutdown",
    ] {
        let mut rig = playing_rig().await?;
        rig.enqueue(B).await?;
        rig.until(|s| s.queue.len() == 1).await?;
        let track = rig.fake.track(0)?;
        let gate = track.seek_gate();
        let old_id = rig.state().track_id;
        request(&rig, 40).await?;
        rig.until(|s| matches!(s.seek, SeekStatus::Pending(_)))
            .await?;
        eventually(|| !track.seeks().is_empty()).await?;
        match action {
            "next" => rig.send(Command::Next).await?,
            "stop" => rig.send(Command::Stop).await?,
            "play" => rig.play(C).await?,
            "leave" => rig.send(Command::LeaveVoice).await?,
            "end" => track.end(),
            "disconnect" => {
                rig.fake.health.send_modify(|h| {
                    h.voice = false;
                    h.voice_error = Some("Disconnected".into());
                });
            }
            "shutdown" => {
                rig.shutdown.send_replace(true);
            }
            _ => unreachable!(),
        }
        if action == "shutdown" {
            rig.task.await?;
            assert!(gate.is_closed());
            continue;
        }
        rig.until(|s| s.track_id != old_id).await?;
        eventually(|| gate.is_closed()).await?;
        assert!(
            gate.send(Ok(Duration::from_secs(40))).is_err(),
            "late result cannot be delivered: {action}"
        );
        rig.send(Command::Seek {
            track_id: old_id,
            position: Duration::from_secs(30),
        })
        .await?;
        rig.settle().await;
        assert!(
            !matches!(
                rig.state().seek,
                SeekStatus::Pending(_) | SeekStatus::Complete(_)
            ),
            "{action}"
        );
        if rig.fake.started() > 1 {
            assert_eq!(rig.fake.track(1)?.seeks(), [] as [Duration; 0]);
        }
        rig.shutdown.send_replace(true);
        rig.task.await?;
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn seek_timeout_is_bounded_and_transport_remains_available() -> Result<()> {
    let mut rig = playing_rig().await?;
    let track = rig.fake.track(0)?;
    let gate = track.seek_gate();
    request(&rig, 40).await?;
    rig.until(|s| matches!(s.seek, SeekStatus::Pending(_)))
        .await?;
    tokio::time::advance(SEEK_TIMEOUT + Duration::from_millis(1)).await;
    let state = rig
        .until(|s| matches!(s.seek, SeekStatus::Failed(_)))
        .await?;
    assert_eq!(state.position.as_secs(), 10);
    assert!(state.can_pause() && state.can_stop());
    assert!(gate.is_closed());
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn seek_to_end_clamps_and_repeats_only_after_one_genuine_end() -> Result<()> {
    for repeat in [RepeatMode::Track, RepeatMode::Queue] {
        let mut rig = playing_rig().await?;
        rig.send(Command::Repeat(repeat)).await?;
        request(&rig, u64::MAX).await?;
        rig.until(|s| s.seek == SeekStatus::Complete(Duration::from_secs(59)))
            .await?;
        assert_eq!(rig.fake.started(), 1);
        rig.fake.track(0)?.end();
        if repeat == RepeatMode::Track {
            rig.started(2).await?;
        } else {
            rig.until(|s| s.phase == Phase::Ready).await?;
        }
        rig.settle().await;
        assert_eq!(
            rig.fake.started(),
            if repeat == RepeatMode::Track { 2 } else { 1 }
        );
        rig.shutdown.send_replace(true);
        rig.task.await?;
    }
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn import_volume_history_persistence_and_queue_repeat_are_independent_of_seek() -> Result<()>
{
    let mut rig = Rig::start(Saved {
        remember_queue: true,
        repeat: RepeatMode::Queue,
        continue_on_error: true,
        ..Saved::default()
    });
    rig.until(|s| s.phase == Phase::Ready).await?;
    rig.enqueue(A).await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 2).await?;
    rig.send(Command::Next).await?;
    rig.started(1).await?;
    let track = rig.fake.track(0)?;
    track.advance(10);
    rig.until(|s| s.phase == Phase::Playing).await?;
    let gate = track.seek_gate();
    request(&rig, 30).await?;
    rig.until(|s| matches!(s.seek, SeekStatus::Pending(_)))
        .await?;
    let import = rig.fake.import_reply();
    rig.send(Command::ImportPlaylist(
        "https://www.youtube.com/playlist?list=PLexample".into(),
    ))
    .await?;
    rig.until(|s| s.import == ImportStatus::Importing).await?;
    import
        .send(Ok(crate::playlist::Playlist {
            entries: vec![crate::playlist::Entry {
                url: C.into(),
                title: Some("C".into()),
            }],
            unavailable: 0,
            limited: false,
        }))
        .map_err(|_| anyhow::anyhow!("import cancelled by seek"))?;
    rig.until(|s| matches!(s.import, ImportStatus::Finished { added: 1, .. }))
        .await?;
    rig.set_volume(40, true);
    rig.until(|s| s.volume == 40).await?;
    assert!(matches!(rig.state().seek, SeekStatus::Pending(_)));
    gate.send(Ok(Duration::from_secs(30)))
        .map_err(|_| anyhow::anyhow!("seek cancelled by import"))?;
    rig.until(|s| matches!(s.seek, SeekStatus::Complete(_)))
        .await?;
    assert_eq!(rig.state().history.len(), 1);
    assert_eq!(rig.queued(), [B, C]);
    track.end();
    rig.started(2).await?;
    rig.until(|s| s.queue.len() == 2 && s.queue[1].url == A)
        .await?;
    assert_eq!(rig.queued(), [C, A]);
    assert!((rig.fake.track(1)?.start_volume - 0.4).abs() < f32::EPSILON);
    let saved: Vec<_> = rig
        .saves
        .borrow()
        .queue
        .iter()
        .map(|item| item.url.clone())
        .collect();
    assert_eq!(saved, [C, A]);
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_valid_slow_range_does_not_trigger_recovery_or_consume_the_queue() -> Result<()> {
    let mut rig = playing_rig().await?;
    rig.enqueue(B).await?;
    rig.until(|s| s.queue.len() == 1).await?;
    let track = rig.fake.track(0)?;
    track.info_hung.store(true, SeqCst);
    tokio::time::advance(Duration::from_millis(300)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    rig.settle().await;
    assert_eq!(rig.state().phase, Phase::Playing);
    assert_eq!(rig.state().position.as_secs(), 10);
    assert!(!track.stopped.load(SeqCst));
    assert_eq!(rig.queued(), [B]);
    track.info_hung.store(false, SeqCst);
    track.advance(11);
    rig.until(|s| s.position.as_secs() == 11).await?;
    assert_eq!(rig.fake.started(), 1);
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn genuinely_unresponsive_driver_is_still_bounded() -> Result<()> {
    let mut rig = playing_rig().await?;
    rig.fake.track(0)?.info_hung.store(true, SeqCst);
    tokio::time::advance(UNRESPONSIVE_TIMEOUT + Duration::from_secs(3)).await;
    rig.until(|s| s.phase == Phase::Error).await?;
    assert!(
        rig.state()
            .error
            .is_some_and(|error| error.contains("stopped responding"))
    );
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn failed_range_end_never_repeats_or_rotates_and_only_opt_in_recovery_advances() -> Result<()>
{
    for repeat in [RepeatMode::Track, RepeatMode::Queue] {
        for continue_on_error in [false, true] {
            let (mut controller, fake, updates) = solo(Saved {
                continue_on_error,
                repeat,
                ..Saved::default()
            });
            let track = track_of(&fake).await?;
            let mut current = active(&track);
            current.confirmed = true;
            current.recorded = true;
            controller.track = Some(current);
            controller.queued_track = true;
            controller.enqueue(B);
            let source = crate::media::SourceControl::default();
            source.record_failure();
            // Includes the MP4 path where seek succeeds before the following packet read fails.
            controller.track.as_mut().context("no track")?.seek =
                SeekStatus::Complete(Duration::from_secs(30));
            let ended = crate::live::source_state(info(PlayMode::End, 30.), Some(&source));
            assert!(
                !controller.apply(Some(Ok(ended))),
                "failure must not request natural_end"
            );
            assert_eq!(updates.borrow().phase, Phase::Error);
            assert_eq!(controller.queue.snapshot().len(), 1);
            assert_eq!(fake.started(), 1);
            assert_eq!(controller.failures, u8::from(continue_on_error));
            assert_eq!(controller.scheduled.is_some(), continue_on_error);
        }
    }
    Ok(())
}
