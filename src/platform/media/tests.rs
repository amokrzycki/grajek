use super::*;
use crate::{
    favorites::{FavoriteEntry, FavoriteRefusal, FavoriteResult},
    media::TrackInfo,
    player::{MediaCommand, Phase, SeekStatus},
    queue::QueueItem,
};

fn playing() -> State {
    State {
        phase: Phase::Playing,
        available: true,
        controller_ready: true,
        voice_connected: true,
        track_id: 42,
        track: Some(TrackInfo {
            title: Some("Real title".into()),
            duration: Some(Duration::from_secs(60)),
            seekable: true,
        }),
        position: Duration::from_secs(10),
        ..State::default()
    }
}

#[test]
fn publishes_only_confirmed_playback_and_truthful_capabilities() {
    let mut state = playing();
    let snapshot = Snapshot::from(&state);
    assert_eq!(snapshot.status, Status::Playing);
    assert_eq!(snapshot.title.as_deref(), Some("Real title"));
    assert!(snapshot.can_pause && snapshot.can_seek);
    assert!(!snapshot.can_next);
    state.seek = SeekStatus::Pending(Duration::from_secs(40));
    assert_eq!(Snapshot::from(&state).position, Duration::from_secs(10));
    for phase in [
        Phase::Ready,
        Phase::Connecting,
        Phase::Resolving,
        Phase::Error,
        Phase::Disconnecting,
    ] {
        state.phase = phase;
        let snapshot = Snapshot::from(&state);
        assert_eq!(snapshot.status, Status::Stopped);
        assert_eq!(snapshot.track_id, None);
        assert_eq!(snapshot.title, None);
        assert_eq!(snapshot.position, Duration::ZERO);
        assert!(!snapshot.can_seek);
    }
    state = playing();
    state.voice_connected = false;
    state.phase = Phase::Paused;
    assert!(!Snapshot::from(&state).can_play);
    state = playing();
    state.available = false;
    let snapshot = Snapshot::from(&state);
    assert_eq!(snapshot.status, Status::Stopped);
    assert!(!snapshot.can_play && !snapshot.can_pause && !snapshot.can_seek);
}

#[test]
fn transport_flags_follow_the_published_capabilities() {
    let mut state = State {
        phase: Phase::Ready,
        available: true,
        controller_ready: true,
        ..State::default()
    };
    let snapshot = Snapshot::from(&state);
    assert!(!snapshot.play_enabled() && !snapshot.pause_enabled());
    state.queue.push(QueueItem {
        id: 1,
        url: "unused".into(),
        title: None,
        title_unavailable: false,
    });
    let snapshot = Snapshot::from(&state);
    assert!(snapshot.play_enabled() && !snapshot.pause_enabled());
    state = playing();
    let snapshot = Snapshot::from(&state);
    assert!(snapshot.play_enabled() && snapshot.pause_enabled());
    state.phase = Phase::Paused;
    let snapshot = Snapshot::from(&state);
    assert!(snapshot.play_enabled() && snapshot.pause_enabled());
    state.voice_connected = false;
    let snapshot = Snapshot::from(&state);
    assert_eq!(snapshot.status, Status::Stopped);
    assert!(!snapshot.play_enabled() && !snapshot.pause_enabled());
}

#[test]
fn endpoint_is_bounded_and_reports_shutdown() {
    let (tx, mut rx) = mpsc::channel(1);
    let (_states, states) = watch::channel(playing());
    let endpoint = Endpoint {
        commands: tx,
        states,
    };
    assert!(endpoint.send(MediaCommand::Pause).is_ok());
    assert!(endpoint.send(MediaCommand::Stop).is_err());
    assert!(matches!(
        rx.try_recv(),
        Ok(player::Command::Media(MediaCommand::Pause))
    ));
    drop(rx);
    assert!(endpoint.send(MediaCommand::Play).is_err());
}

#[test]
fn search_and_favorites_snapshots_do_not_change_native_media_or_tray_projection() {
    let mut state = playing();
    state.favorites.push(FavoriteEntry {
        url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
        title: Some("Kept".into()),
        channel: None,
        duration_secs: Some(60),
        live: false,
    });
    state.favorite_result = Some(FavoriteResult {
        request_id: 1,
        url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
        saved: true,
        result: Ok(()),
    });
    state.current_url = Some("https://www.youtube.com/watch?v=aaaaaaaaaaa".into());
    let media = Snapshot::from(&state);
    let tray = crate::platform::tray::menu(&state);
    for phase in [
        crate::search::SearchPhase::Loading,
        crate::search::SearchPhase::Empty { rejected: 2 },
        crate::search::SearchPhase::Failed(crate::search::Error::Unavailable),
        crate::search::SearchPhase::Results(crate::search::Results {
            items: Vec::new(),
            rejected: 0,
        }),
    ] {
        state.search = crate::search::SearchState {
            id: 10,
            query: "Music".into(),
            phase,
            status: None,
        };
        assert_eq!(Snapshot::from(&state), media);
        assert_eq!(crate::platform::tray::menu(&state), tray);
    }
    assert_eq!(
        state
            .favorite_result
            .as_ref()
            .map(|result| result.request_id),
        Some(1)
    );
    state.favorites.clear();
    state.current_url = None;
    state.favorite_result = Some(FavoriteResult {
        request_id: 2,
        url: "https://www.youtube.com/watch?v=bbbbbbbbbbb".into(),
        saved: true,
        result: Err(FavoriteRefusal::Full),
    });
    assert_eq!(Snapshot::from(&state), media);
    assert_eq!(crate::platform::tray::menu(&state), tray);
    state.favorite_result = Some(FavoriteResult {
        request_id: 3,
        url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
        saved: false,
        result: Ok(()),
    });
    assert_eq!(Snapshot::from(&state), media);
    assert_eq!(crate::platform::tray::menu(&state), tray);
}
