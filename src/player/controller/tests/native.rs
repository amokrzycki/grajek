use super::*;
use crate::native::Command as Media;

#[tokio::test(start_paused = true)]
async fn external_play_is_idempotent_and_stop_keeps_pending_queue_during_resolution() -> Result<()>
{
    let mut rig = Rig::new().await?;
    rig.fake.hang(A);
    for url in [A, B] {
        rig.enqueue(url).await?;
    }
    rig.until(|state| state.queue.len() == 2).await?;
    for _ in 0..5 {
        rig.send(Command::Media(Media::Play)).await?;
    }
    rig.send(Command::RememberQueue(true)).await?;
    rig.until(|state| state.preferences.remember_queue && state.phase == Phase::Resolving)
        .await?;
    assert_eq!(rig.fake.requests(), [A]);
    assert_eq!(rig.queued(), [B]);
    rig.send(Command::Media(Media::Stop)).await?;
    rig.until(|state| state.phase == Phase::Ready).await?;
    assert_eq!(rig.queued(), [B]);
    assert_eq!(rig.fake.cancelled.load(SeqCst), 1);
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn external_pause_resume_toggle_and_stale_seek_use_the_real_controller() -> Result<()> {
    let mut rig = Rig::new().await?;
    rig.play(A).await?;
    rig.started(1).await?;
    let first = rig.fake.track(0)?;
    first.advance(10);
    let old_id = rig
        .until(|state| state.phase == Phase::Playing)
        .await?
        .track_id;
    rig.enqueue(B).await?;
    rig.until(|state| state.queue.len() == 1).await?;
    for _ in 0..3 {
        rig.send(Command::Media(Media::Pause)).await?;
    }
    rig.until(|state| state.phase == Phase::Paused).await?;
    for _ in 0..3 {
        rig.send(Command::Media(Media::Play)).await?;
    }
    rig.send(Command::RememberQueue(true)).await?;
    rig.until(|state| state.phase == Phase::Playing && state.preferences.remember_queue)
        .await?;
    assert_eq!(first.calls(), ["pause", "resume"]);
    assert_eq!(rig.queued(), [B]);
    rig.send(Command::Media(Media::PlayPause)).await?;
    rig.until(|state| state.phase == Phase::Paused).await?;
    rig.send(Command::Media(Media::PlayPause)).await?;
    rig.until(|state| state.phase == Phase::Playing).await?;
    let gate = first.seek_gate();
    rig.send(Command::Media(Media::Seek {
        track_id: old_id,
        position: Duration::from_secs(30),
    }))
    .await?;
    rig.until(|state| matches!(state.seek, SeekStatus::Pending(_)))
        .await?;
    assert_eq!(rig.state().position, Duration::from_secs(10));
    rig.send(Command::Media(Media::Next)).await?;
    rig.started(2).await?;
    assert!(gate.is_closed());
    rig.fake.track(1)?.advance(2);
    rig.until(|state| state.phase == Phase::Playing).await?;
    rig.send(Command::Media(Media::Seek {
        track_id: old_id,
        position: Duration::from_secs(40),
    }))
    .await?;
    rig.send(Command::RememberQueue(false)).await?;
    rig.until(|state| !state.preferences.remember_queue).await?;
    assert_eq!(rig.fake.track(1)?.seeks(), Vec::<Duration>::new());
    assert_eq!(rig.state().position, Duration::from_secs(2));
    rig.shutdown.send_replace(true);
    rig.task.await?;
    Ok(())
}
