mod update;
mod view;

pub(in crate::app) use update::RemoteCheck;

use crate::media::components::{
    self, Need,
    download::{self, Download, Progress},
    status::{ComponentReport, ComponentState},
    system_override,
};
use gpui_kit::{Context, FocusHandle, Focusable, ScrollHandle, Task, Window};

use super::PlayerApp;

pub(super) fn opens_at_startup(need: Option<Need>) -> bool {
    matches!(need, Some(Need::Missing | Need::Repair))
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Open, disclosure and pending focus/check are independent UI concerns"
)]
pub(super) struct Panel {
    pub open: bool,
    pub check_pending: bool,
    pub reports: Vec<ComponentReport>,
    pub progress: Progress,
    pub remote: RemoteCheck,
    pub details: bool,
    focus_pending: bool,
    focus: FocusHandle,
    scroll: ScrollHandle,
    runtime: Option<tokio::runtime::Handle>,
    job: Option<Download>,
    updates: Option<Task<()>>,
    check: Option<Task<()>>,
    check_abort: Option<tokio::task::AbortHandle>,
    generation: u64,
    remote_generation: u64,
}

impl Panel {
    pub fn new(cx: &Context<PlayerApp>) -> Self {
        Self {
            open: false,
            check_pending: false,
            reports: components::tools().inspect(),
            progress: Progress::Ready,
            remote: RemoteCheck::Idle,
            details: false,
            focus_pending: false,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            runtime: None,
            job: None,
            updates: None,
            check: None,
            check_abort: None,
            generation: 0,
            remote_generation: 0,
        }
    }

    fn invalidate_check(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(abort) = self.check_abort.take() {
            abort.abort();
        }
        self.check = None;
    }

    fn ready(&self) -> bool {
        self.reports.len() == 2
            && self
                .reports
                .iter()
                .all(|report| report.state == ComponentState::Ready)
    }

    fn busy(&self) -> bool {
        self.job.is_some()
            || matches!(
                self.progress,
                Progress::Downloading { .. } | Progress::Extracting | Progress::Checking
            )
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.invalidate_check();
    }
}

impl PlayerApp {
    pub(super) fn components_busy(&self) -> bool {
        self.components.busy()
            || self
                .components
                .reports
                .iter()
                .any(|r| r.state == ComponentState::Checking)
    }
    pub fn with_components(mut self, runtime: tokio::runtime::Handle) -> Self {
        self.components.runtime = Some(runtime);
        self.components.open = opens_at_startup(components::tools().need());
        self.components.focus_pending = self.components.open;
        self.components.check_pending = true;
        self
    }

    pub(super) fn components_action(&self) -> &'static str {
        if self.components.busy() {
            return "Download progress";
        }
        if matches!(self.components.remote, RemoteCheck::Available { .. }) {
            return "Update components";
        }
        if self
            .components
            .reports
            .iter()
            .any(|r| matches!(r.state, ComponentState::NeedsRepair(_)))
        {
            "Repair components"
        } else if self
            .components
            .reports
            .iter()
            .any(|r| r.state == ComponentState::Missing)
        {
            "Download missing components"
        } else if components::tools().need() == Some(Need::Update) {
            "Update components"
        } else {
            "Repair components"
        }
    }

    pub(super) fn check_components(&mut self, cx: &mut Context<Self>) {
        let startup = std::mem::take(&mut self.components.check_pending);
        if self.components.busy() {
            return;
        }
        let Some(runtime) = self.components.runtime.clone() else {
            return;
        };
        self.components.invalidate_check();
        let generation = self.components.generation;
        for report in &mut self.components.reports {
            report.state = ComponentState::Checking;
        }
        let job = runtime.spawn(async { components::tools().check().await });
        self.components.check_abort = Some(job.abort_handle());
        self.components.check = Some(cx.spawn(async move |entity, cx| {
            let result = job.await;
            let _ = entity.update(cx, |this, cx| {
                if generation != this.components.generation {
                    return;
                }
                this.components.check_abort = None;
                match result {
                    Ok(reports) => this.components.reports = reports,
                    Err(_) => {
                        for report in &mut this.components.reports {
                            report.state = ComponentState::NeedsRepair(
                                "The component check was interrupted. Try checking again.".into(),
                            );
                        }
                    }
                }
                // Presence checks cannot detect an executable that fails to run.
                if this.components.reports.iter().any(|r| {
                    matches!(
                        r.state,
                        ComponentState::Missing | ComponentState::NeedsRepair(_)
                    )
                }) && startup
                {
                    this.components.open = true;
                    this.components.focus_pending = true;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn open_components(&mut self, cx: &mut Context<Self>) {
        self.retire_search(cx);
        self.components.open = true;
        self.components.focus_pending = true;
        self.help.dismiss(cx);
        self.close_popups();
        if !self.components.busy() {
            self.check_components(cx);
        }
        cx.notify();
    }

    fn close_components(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.components.open = false;
        if self.components.check_abort.is_some() {
            self.components.invalidate_check();
            self.components.reports = components::tools().inspect();
        }
        if self.settings_open {
            self.focus_settings(window, cx);
        } else {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    fn download_components(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.components.job.is_some() {
            return;
        }
        if system_override() {
            self.components.progress = Progress::Failed("GRAJEK_TOOLS=system prevents Grajek from using downloaded components. Remove it and restart Grajek, or repair your system tools.".into());
            cx.notify();
            return;
        }
        self.components.invalidate_check();
        let result = (|| {
            let runtime = self.components.runtime.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Download runtime is unavailable. Restart Grajek.")
            })?;
            let directory = crate::paths::tools_dir().ok_or_else(|| {
                anyhow::anyhow!("No writable application data directory is available.")
            })?;
            let pins =
                components::effective_pins(crate::paths::component_manifest_dir().as_deref())?;
            download::start(runtime, directory, pins)
        })();
        let job = match result {
            Ok(job) => job,
            Err(error) => {
                self.components.progress = Progress::Failed(error.to_string());
                cx.notify();
                return;
            }
        };
        let mut updates = job.progress.clone();
        self.components.remote = RemoteCheck::Idle;
        self.components.job = Some(job);
        self.components.progress = Progress::Ready;
        self.components.updates = Some(cx.spawn_in(window, async move |entity, cx| {
            while updates.changed().await.is_ok() {
                let progress = updates.borrow_and_update().clone();
                let complete = progress == Progress::Complete;
                let done = matches!(
                    progress,
                    Progress::Complete | Progress::Cancelled | Progress::Failed(_)
                );
                if entity
                    .update_in(cx, |this, _, cx| {
                        if done {
                            this.components.job = None;
                        }
                        this.components.progress = progress;
                        if complete {
                            this.check_components(cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                    || done
                {
                    break;
                }
            }
        }));
        cx.notify();
    }
}
