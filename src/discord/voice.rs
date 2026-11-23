use serenity::async_trait;
use songbird::{
    Event, EventContext, EventHandler as VoiceEventHandler, error::JoinError,
    events::context_data::DisconnectReason, model::CloseCode,
};
use tokio::sync::watch;

use super::health::Health;

pub(super) fn retryable(reason: Option<DisconnectReason>) -> bool {
    matches!(
        reason,
        Some(
            DisconnectReason::Io
                | DisconnectReason::TimedOut
                | DisconnectReason::WsClosed(Some(
                    CloseCode::SessionInvalid
                        | CloseCode::SessionTimeout
                        | CloseCode::VoiceServerCrash
                ))
        )
    )
}

pub(crate) fn retryable_join(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<JoinError>(),
        Some(JoinError::Driver(error)) if retryable(Some(DisconnectReason::from(error)))
    )
}

pub(super) struct VoiceEvents {
    pub(super) health: watch::Sender<Health>,
    pub(super) epoch: u64,
}

impl VoiceEvents {
    pub(super) fn update(
        &self,
        session: &str,
        connected: bool,
        reconnected: bool,
        reason: Option<DisconnectReason>,
    ) {
        self.health.send_if_modified(|health| {
            if health.voice_epoch != self.epoch || !health.voice_requested {
                return false;
            }
            if !connected && health.voice_session.as_deref().is_some_and(|current| current != session) {
                return false;
            }
            // Requested/discarded callbacks can belong to a superseded driver attempt.
            if !connected && matches!(reason, None | Some(DisconnectReason::Requested | DisconnectReason::AttemptDiscarded)) {
                return false;
            }
            health.voice_session = Some(session.into());
            health.voice = connected;
            health.voice_retry = !connected && retryable(reason);
            health.voice_error = if reconnected {
                Some("Discord voice restored. Playback was interrupted; press Next or Play URL.".into())
            } else if connected {
                None
            } else {
                Some("Discord voice disconnected. Check the network and permissions; press Next or Play URL.".into())
            };
            true
        });
    }
}

#[async_trait]
impl VoiceEventHandler for VoiceEvents {
    async fn act(&self, event: &EventContext<'_>) -> Option<Event> {
        match event {
            EventContext::DriverConnect(data) => self.update(data.session_id, true, false, None),
            EventContext::DriverReconnect(data) => self.update(data.session_id, true, true, None),
            EventContext::DriverDisconnect(data) => {
                // Never log the voice session ID or the entire event context.
                tracing::warn!(kind = ?data.kind, reason = ?data.reason, "Discord voice disconnected");
                self.update(data.session_id, false, false, data.reason);
            }
            _ => {}
        }
        None
    }
}
