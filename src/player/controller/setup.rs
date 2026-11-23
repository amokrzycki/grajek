use super::Controller;
use crate::player::{Backend, Phase};
use anyhow::Result;
use tokio::task::JoinError;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SetupChange {
    Discovery,
    Reconfigure,
    Remove,
}

impl<B: Backend> Controller<B> {
    pub(super) fn setup_interrupting(&self) -> bool {
        self.setup_job.is_some() && self.setup_change != SetupChange::Discovery
    }

    pub(super) async fn setup_command(&mut self, request: crate::discord::setup::Request) {
        if self.setup_job.is_some() {
            return self.sync();
        }
        if request.interrupts() && (self.track.is_some() || self.job.is_some()) {
            self.pending_setup = Some(request);
            self.sync();
        } else {
            self.configure(request).await;
        }
    }

    pub(super) async fn confirm_setup(&mut self, accept: bool) {
        if let Some(request) = self.pending_setup.take()
            && accept
        {
            self.configure(request).await;
        }
        self.sync();
    }

    pub(in crate::player) async fn configure(&mut self, request: crate::discord::setup::Request) {
        if self.setup_job.is_some() {
            return self.sync();
        }
        self.setup_change = if matches!(request, crate::discord::setup::Request::Remove) {
            SetupChange::Remove
        } else if request.interrupts() {
            SetupChange::Reconfigure
        } else {
            SetupChange::Discovery
        };
        if self.setup_change != SetupChange::Discovery {
            self.left_voice = true;
            self.scheduled = None;
            self.cancel_job().await;
            self.stop_track();
            self.failure_notice = None;
            self.publish(Phase::Ready, None);
        }
        let backend = self.backend.clone();
        self.setup_job = Some(tokio::spawn(async move { backend.setup(request).await }));
        self.sync();
    }

    pub(super) async fn finish_setup(&mut self, result: &Result<Result<()>, JoinError>) {
        self.setup_job = None;
        // A health update during discovery may have deferred voice cleanup.
        if !self.backend.ready() && self.health.borrow().voice {
            self.leave_voice(true).await;
            return;
        }
        let succeeded = matches!(result, Ok(Ok(())));
        if succeeded && self.setup_change != SetupChange::Discovery {
            let setup = self.health.borrow().setup.clone();
            if self.setup_change == SetupChange::Remove {
                self.saved.destination = None;
                self.persist();
            } else if let Some(destination) = setup.destination {
                self.saved.destination = Some(destination);
                self.persist();
            } else if let Some(destination) = self
                .saved
                .destination
                .clone()
                .filter(|destination| Some(destination.bot) == setup.bot)
            {
                self.configure(crate::discord::setup::Request::Select(destination))
                    .await;
                return;
            }
        }
        if !succeeded {
            self.report("Discord setup did not finish. Check Settings and retry.".into());
        }
        self.sync();
    }
}
