use super::*;
use crate::{
    media::TrackInfo,
    player::{FavoriteRefusal, FavoriteResult, SeekStatus},
    queue::QueueItem,
    store::FavoriteEntry,
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
    assert!(translate(Command::Play, &state).is_none());
    state = playing();
    state.available = false;
    let snapshot = Snapshot::from(&state);
    assert_eq!(snapshot.status, Status::Stopped);
    assert!(!snapshot.can_play && !snapshot.can_pause && !snapshot.can_seek);
}

#[test]
fn play_never_invents_media_and_seek_needs_a_current_seekable_track() {
    let mut state = State {
        phase: Phase::Ready,
        available: true,
        controller_ready: true,
        ..State::default()
    };
    assert!(translate(Command::Play, &state).is_none());
    state.queue.push(QueueItem {
        id: 1,
        url: "unused".into(),
        title: None,
        title_unavailable: false,
    });
    assert!(Snapshot::from(&state).can_play);
    assert!(matches!(
        translate(Command::Play, &state),
        Some(player::Command::Next)
    ));
    state = playing();
    assert!(translate(Command::Play, &state).is_none());
    assert!(matches!(
        translate(Command::PlayPause, &state),
        Some(player::Command::Pause)
    ));
    state.phase = Phase::Paused;
    assert!(matches!(
        translate(Command::Play, &state),
        Some(player::Command::Resume)
    ));
    assert!(translate(Command::Pause, &state).is_none());
    assert!(
        translate(
            Command::Seek {
                track_id: 41,
                position: Duration::ZERO
            },
            &state
        )
        .is_none()
    );
    assert!(matches!(
        translate(
            Command::Seek {
                track_id: 42,
                position: Duration::from_secs(20)
            },
            &state
        ),
        Some(player::Command::Seek { track_id: 42, .. })
    ));
    if let Some(track) = &mut state.track {
        track.seekable = false;
    }
    assert!(!Snapshot::from(&state).can_seek);
    assert!(
        translate(
            Command::Seek {
                track_id: 42,
                position: Duration::ZERO
            },
            &state
        )
        .is_none()
    );
    if let Some(track) = &mut state.track {
        track.seekable = true;
        track.duration = None;
    }
    assert!(!Snapshot::from(&state).can_seek);
    assert_eq!(Snapshot::from(&state).duration, None);
}

#[test]
fn relative_seek_is_saturating_and_beyond_end_uses_existing_next() {
    let mut state = playing();
    assert!(matches!(
        translate(
            Command::SeekRelative {
                track_id: 42,
                offset: i64::MIN
            },
            &state
        ),
        Some(player::Command::Seek {
            position: Duration::ZERO,
            ..
        })
    ));
    assert!(
        translate(
            Command::SeekRelative {
                track_id: 42,
                offset: i64::MAX
            },
            &state
        )
        .is_none()
    );
    state.queue.push(QueueItem {
        id: 1,
        url: "unused".into(),
        title: None,
        title_unavailable: false,
    });
    assert!(matches!(
        translate(
            Command::SeekRelative {
                track_id: 42,
                offset: i64::MAX
            },
            &state
        ),
        Some(player::Command::Next)
    ));
    assert!(
        translate(
            Command::Seek {
                track_id: 42,
                position: Duration::from_secs(61)
            },
            &state
        )
        .is_none()
    );
    assert!(matches!(
        translate(Command::Stop, &state),
        Some(player::Command::Stop)
    ));
    assert!(matches!(
        translate(Command::Next, &state),
        Some(player::Command::Next)
    ));
}

#[test]
fn endpoint_is_bounded_and_reports_shutdown() {
    let (tx, mut rx) = mpsc::channel(1);
    let (_states, states) = watch::channel(playing());
    let endpoint = Endpoint {
        commands: tx,
        states,
    };
    assert!(endpoint.send(Command::Pause).is_ok());
    assert!(endpoint.send(Command::Stop).is_err());
    assert!(matches!(
        rx.try_recv(),
        Ok(player::Command::Media(Command::Pause))
    ));
    drop(rx);
    assert!(endpoint.send(Command::Play).is_err());
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
    let tray = crate::tray::menu(&state);
    for phase in [
        crate::player::SearchPhase::Loading,
        crate::player::SearchPhase::Empty { rejected: 2 },
        crate::player::SearchPhase::Failed(crate::search::Error::Unavailable),
        crate::player::SearchPhase::Results(crate::search::Results {
            items: Vec::new(),
            rejected: 0,
        }),
    ] {
        state.search = crate::player::SearchState {
            id: 10,
            query: "Music".into(),
            phase,
            status: None,
        };
        assert_eq!(Snapshot::from(&state), media);
        assert_eq!(crate::tray::menu(&state), tray);
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
    assert_eq!(crate::tray::menu(&state), tray);
    state.favorite_result = Some(FavoriteResult {
        request_id: 3,
        url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
        saved: false,
        result: Ok(()),
    });
    assert_eq!(Snapshot::from(&state), media);
    assert_eq!(crate::tray::menu(&state), tray);
}
