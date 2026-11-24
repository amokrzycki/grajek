use super::*;
use crate::{media::TrackInfo, queue::QueueItem};

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
fn media_projection_publishes_only_confirmed_playback_and_truthful_capabilities() {
    let mut state = playing();
    assert_eq!(state.media_status(), Status::Playing);
    assert_eq!(state.media_track_id(), Some(42));
    assert_eq!(state.media_duration(), Some(Duration::from_secs(60)));
    assert_eq!(state.media_position(), Duration::from_secs(10));
    assert!(state.media_can_pause() && state.media_can_seek() && !state.can_next());
    for phase in [
        Phase::Ready,
        Phase::Connecting,
        Phase::Resolving,
        Phase::Error,
        Phase::Disconnecting,
    ] {
        state.phase = phase;
        assert_eq!(state.media_status(), Status::Stopped);
        assert_eq!(state.media_track_id(), None);
        assert_eq!(state.media_duration(), None);
        assert_eq!(state.media_position(), Duration::ZERO);
        assert!(!state.media_can_seek());
    }
    state = playing();
    state.voice_connected = false;
    state.phase = Phase::Paused;
    assert!(!state.media_can_play());
    assert!(translate(MediaCommand::Play, &state).is_none());
    state = playing();
    state.available = false;
    assert_eq!(state.media_status(), Status::Stopped);
    assert!(!state.media_can_play() && !state.media_can_pause() && !state.media_can_seek());
}

#[test]
fn play_never_invents_media_and_seek_needs_a_current_seekable_track() {
    let mut state = State {
        phase: Phase::Ready,
        available: true,
        controller_ready: true,
        ..State::default()
    };
    assert!(translate(MediaCommand::Play, &state).is_none());
    state.queue.push(QueueItem {
        id: 1,
        url: "unused".into(),
        title: None,
        title_unavailable: false,
    });
    assert!(state.media_can_play());
    assert!(matches!(
        translate(MediaCommand::Play, &state),
        Some(Command::Next)
    ));
    state = playing();
    assert!(translate(MediaCommand::Play, &state).is_none());
    assert!(matches!(
        translate(MediaCommand::PlayPause, &state),
        Some(Command::Pause)
    ));
    state.phase = Phase::Paused;
    assert!(matches!(
        translate(MediaCommand::Play, &state),
        Some(Command::Resume)
    ));
    assert!(translate(MediaCommand::Pause, &state).is_none());
    assert!(
        translate(
            MediaCommand::Seek {
                track_id: 41,
                position: Duration::ZERO
            },
            &state
        )
        .is_none()
    );
    assert!(matches!(
        translate(
            MediaCommand::Seek {
                track_id: 42,
                position: Duration::from_secs(20)
            },
            &state
        ),
        Some(Command::Seek { track_id: 42, .. })
    ));
    if let Some(track) = &mut state.track {
        track.seekable = false;
    }
    assert!(!state.media_can_seek());
    assert!(
        translate(
            MediaCommand::Seek {
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
    assert!(!state.media_can_seek());
    assert_eq!(state.media_duration(), None);
}

#[test]
fn relative_seek_is_saturating_and_beyond_end_uses_existing_next() {
    let mut state = playing();
    assert!(matches!(
        translate(
            MediaCommand::SeekRelative {
                track_id: 42,
                offset: i64::MIN
            },
            &state
        ),
        Some(Command::Seek {
            position: Duration::ZERO,
            ..
        })
    ));
    assert!(
        translate(
            MediaCommand::SeekRelative {
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
            MediaCommand::SeekRelative {
                track_id: 42,
                offset: i64::MAX
            },
            &state
        ),
        Some(Command::Next)
    ));
    assert!(
        translate(
            MediaCommand::Seek {
                track_id: 42,
                position: Duration::from_secs(61)
            },
            &state
        )
        .is_none()
    );
    assert!(matches!(
        translate(MediaCommand::Stop, &state),
        Some(Command::Stop)
    ));
    assert!(matches!(
        translate(MediaCommand::Next, &state),
        Some(Command::Next)
    ));
}
