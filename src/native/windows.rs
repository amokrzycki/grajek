//! UI-thread SMTC lifetime; native callbacks touch only a bounded command sender.
use super::{Command, Endpoint, Snapshot, Status};
use gpui_kit::{App, Task, Window};
use grajek_smtc::{Event, MediaStatus, Session, View};

pub struct MediaSession {
    _updates: Task<()>,
}

pub fn attach(window: &Window, cx: &App, endpoint: &Endpoint) -> Option<MediaSession> {
    let callback_endpoint = endpoint.clone();
    let mut session = match Session::new(window, move |event| {
        let command = match event {
            Event::Play => Command::Play,
            Event::Pause => Command::Pause,
            Event::Next => Command::Next,
            Event::Stop => Command::Stop,
            Event::Seek { track_id, position } => Command::Seek { track_id, position },
        };
        if let Err(error) = callback_endpoint.send(command) {
            tracing::debug!(error, "SMTC command rejected");
        }
    }) {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(%error, "Windows media controls unavailable; playback remains available");
            return None;
        }
    };
    let mut states = endpoint.states.clone();
    let initial = endpoint.snapshot();
    if let Err(error) = publish(&mut session, &initial) {
        tracing::warn!(%error, "SMTC initial publication failed");
        return None;
    }
    // A Tokio watch is woken by the runtime, while COM work remains on GPUI's STA.
    let updates = cx.spawn(async move |_| {
        let mut before = initial;
        while states.changed().await.is_ok() {
            let after = Snapshot::from(&*states.borrow_and_update());
            if before != after {
                if let Err(error) = publish(&mut session, &after) {
                    tracing::warn!(%error, "SMTC publication failed; session stopped");
                    break;
                }
                before = after;
            }
        }
    });
    Some(MediaSession { _updates: updates })
}

fn publish(session: &mut Session, snapshot: &Snapshot) -> grajek_smtc::Result<()> {
    session.publish(&View {
        track_id: snapshot.track_id,
        title: snapshot.title.as_deref(),
        status: match snapshot.status {
            Status::Stopped => MediaStatus::Stopped,
            Status::Playing => MediaStatus::Playing,
            Status::Paused => MediaStatus::Paused,
        },
        duration: snapshot.duration,
        position: snapshot.position,
        can_play: snapshot.can_play || snapshot.can_pause,
        can_pause: snapshot.can_pause || snapshot.status == Status::Paused,
        can_next: snapshot.can_next,
        can_stop: snapshot.can_stop,
        can_seek: snapshot.can_seek,
    })
}
