use super::jobs::{Job, Origin};
use super::{Controller, Scheduled};
use crate::discord::Health;
use crate::player::{Backend, FAILURE_LIMIT, Phase};
use anyhow::Context;
use std::time::Duration;
use tokio::time::{Instant, timeout};

const REJOIN_LIMIT: u8 = 2;
const SKIP_DELAY: Duration = Duration::from_secs(1);

impl<B: Backend> Controller<B> {
    pub(super) async fn leave_voice(&mut self, manual: bool) {
        if self.manual_disconnect() {
            return self.sync();
        }
        self.left_voice = true;
        self.scheduled = None;
        self.failure_notice = None;
        self.rejoin_attempts = 0;
        self.cancel_job().await;
        self.stop_track();
        // Automatic idle leave remains interruptible by explicit Play/Next.
        if manual {
            self.publish(Phase::Disconnecting, None);
        }
        let backend = self.backend.clone();
        self.job = Some(Job::Disconnect {
            task: tokio::spawn(async move {
                timeout(Duration::from_secs(5), backend.disconnect())
                    .await
                    .context("Leaving voice timed out.")??;
                Ok(())
            }),
            manual,
        });
    }

    pub(super) async fn leave_voice_command(&mut self) {
        if self.setup_job.is_none() {
            self.leave_voice(true).await;
        } else {
            self.sync();
        }
    }

    pub(super) fn media_failure(&mut self, message: &str) {
        let queued = self
            .track
            .as_ref()
            .is_some_and(|track| track.request.origin == Origin::Queue);
        self.media_failure_for(message, queued);
    }

    pub(super) fn media_failure_for(&mut self, message: &str, queued: bool) {
        self.fail(message);
        let healthy = {
            let health = self.health.borrow();
            health.gateway && health.voice && health.error().is_none()
        };
        if !self.saved.continue_on_error || !queued || !healthy {
            return;
        }
        self.failures = self.failures.saturating_add(1);
        let explanation = if self.failures >= FAILURE_LIMIT {
            format!(
                "Recovery stopped after {FAILURE_LIMIT} consecutive failures. Press Next or Play URL."
            )
        } else if self.queue.is_empty() {
            "No queued tracks remain. Press Play URL or add tracks and press Next.".into()
        } else {
            self.scheduled = Some((Instant::now() + SKIP_DELAY, Scheduled::Advance));
            "Skipped failed queued track; continuing in 1 second.".into()
        };
        self.failure_notice = Some(format!("{message} {explanation}"));
        self.publish(Phase::Error, self.failure_notice.clone());
    }

    pub(super) fn schedule_rejoin(&mut self) {
        if self.rejoin_attempts >= REJOIN_LIMIT {
            self.publish(Phase::Error, Some(format!("Discord voice recovery stopped after {REJOIN_LIMIT} attempts. Check the network and permissions, then press Next or Play URL.")));
        } else {
            self.rejoin_attempts += 1;
            self.scheduled = Some((
                Instant::now() + Duration::from_secs(u64::from(self.rejoin_attempts) * 2 - 1),
                Scheduled::Rejoin(self.rejoin_attempts),
            ));
            self.publish(Phase::Error, Some("Discord voice interrupted; reconnecting. Playback will not restart automatically.".into()));
        }
    }

    pub(super) async fn health_changed(&mut self, current: Health) {
        if !self.backend.ready() && current.voice && self.setup_job.is_none() {
            self.leave_voice(true).await;
            return;
        }
        // Explicit leave owns teardown; late voice callbacks cannot cancel it or rejoin.
        if self.left_voice {
            if current.gateway_error.is_some() && self.states.borrow().phase != Phase::Disconnecting
            {
                self.publish(Phase::Error, current.gateway_error);
            } else {
                self.sync();
            }
            return;
        }
        let Some(error) = current.error() else {
            self.sync();
            return;
        };
        // Voice callbacks from a rejoin belong to that job; its result owns the retry budget.
        if current.gateway
            && current.voice_retry
            && (matches!(self.job.as_ref(), Some(Job::Rejoin { .. }))
                || matches!(self.scheduled, Some((_, Scheduled::Rejoin(_)))))
        {
            return;
        }
        self.scheduled = None;
        self.cancel_job().await;
        self.stop_track();
        self.failure_notice = None;
        self.publish(Phase::Error, Some(error));
        if current.gateway && !current.voice && current.voice_retry {
            self.schedule_rejoin();
        }
    }

    /// Cancels pending work and the active track. The queue is kept and nothing advances.
    pub(super) async fn stop(&mut self) {
        self.scheduled = None;
        self.failure_notice = None;
        self.publish(Phase::Stopping, None);
        self.cancel_job().await;
        self.stop_track();
        self.backend.recover().await;
        let error = self.health.borrow().error();
        let phase = if error.is_some() {
            Phase::Error
        } else {
            Phase::Ready
        };
        self.publish(phase, error);
    }

    pub(super) async fn stop_command(&mut self) {
        if self.setup_interrupting() {
            return self.sync();
        }
        if !self.manual_disconnect() {
            self.stop().await;
        }
    }
}
