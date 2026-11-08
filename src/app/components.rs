use super::{
    ActiveTheme, ButtonVariants, Context, Disableable, FluentBuilder, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, PlayerApp,
    StatefulInteractiveElement, Styled, Task, TestSupportExt, Window, div, px,
};
use crate::media::{
    Tool,
    components::{
        self,
        download::{self, Download, Progress},
        status::{ComponentReport, ComponentState},
    },
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{AnyElement, FocusHandle, ScrollHandle};

#[expect(
    clippy::struct_excessive_bools,
    reason = "Open, disclosure and pending focus/check are independent UI concerns"
)]
pub(super) struct Panel {
    pub open: bool,
    pub check_pending: bool,
    pub reports: Vec<ComponentReport>,
    pub progress: Progress,
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
}

impl Panel {
    pub fn new(cx: &Context<PlayerApp>) -> Self {
        Self {
            open: false,
            check_pending: false,
            reports: components::tools().inspect(),
            progress: Progress::Ready,
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
        self.components.open = components::tools().need().is_some();
        self.components.focus_pending = self.components.open;
        self.components.check_pending = true;
        self
    }

    pub(super) fn components_action(&self) -> &'static str {
        if self.components.busy() {
            return "Download progress";
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
        self.components.open = true;
        self.components.focus_pending = true;
        self.help.dismiss(cx);
        self.row_menu = None;
        self.queue_options_open = false;
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
            let directory = crate::store::app_data_dir()
                .ok_or_else(|| {
                    anyhow::anyhow!("No writable application data directory is available.")
                })?
                .join("tools");
            download::start(runtime, directory)
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

    pub(super) fn component_rows(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut rows = div().flex().flex_col().gap_3();
        for report in &self.components.reports {
            let name = match report.tool {
                Tool::YtDlp => "yt-dlp",
                Tool::Deno => "Deno",
            };
            let description = match report.tool {
                Tool::YtDlp => "Finds YouTube audio and playlist information",
                Tool::Deno => "Runs the JavaScript needed by YouTube",
            };
            let id = match report.tool {
                Tool::YtDlp => "component-yt-dlp",
                Tool::Deno => "component-deno",
            };
            let label = format!("{name}: {}", report.state.label());
            let mut row = div()
                .id(id)
                .test_support()
                .aria_label(label)
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(match report.state {
                                    ComponentState::Ready => theme.green,
                                    ComponentState::Missing | ComponentState::NeedsRepair(_) => {
                                        theme.danger
                                    }
                                    _ => theme.muted_foreground,
                                })
                                .child(report.state.label()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.muted_foreground)
                        .child(description),
                );
            if let ComponentState::NeedsRepair(reason) = &report.state {
                row = row.child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.danger)
                        .child(reason.clone()),
                );
            }
            rows = rows.child(row);
        }
        rows.into_any_element()
    }

    fn components_details(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tools = components::tools();
        let mut details = div()
            .id("components-details")
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(theme.muted_foreground);
        if let Ok(pins) = components::pins() {
            let megabytes =
                (pins.ytdlp.artifact().size + pins.deno.artifact().size).div_ceil(1_000_000);
            details = details.child(format!("Downloads the verified pair (about {megabytes} MB) from official GitHub releases. No administrator access or PATH changes."))
                .child(format!("Expected: yt-dlp {}, bundled EJS {}, Deno {}.", pins.ytdlp.version, pins.ytdlp.ejs, pins.deno.version))
                .child(format!("yt-dlp source: {}", pins.ytdlp.artifact().url))
                .child(format!("SHA-256: {}", pins.ytdlp.artifact().sha256))
                .child(format!("Deno source: {}", pins.deno.artifact().url))
                .child(format!("SHA-256: {}", pins.deno.artifact().sha256));
        }
        for report in &self.components.reports {
            let path = tools
                .component_path(report.tool)
                .map_or_else(|| "Not found".into(), |p| p.display().to_string());
            details = details.child(format!(
                "{}: {} · {}",
                report.tool.name(),
                report.version.as_deref().unwrap_or("Version not verified"),
                path
            ));
        }
        if let Some(directory) = crate::store::app_data_dir() {
            details = details.child(format!(
                "Download location: {}",
                directory.join("tools").display()
            ));
        }
        if let Progress::Failed(reason) = &self.components.progress {
            details = details.child(reason.clone());
        }
        details
            .child("Licenses: yt-dlp GPLv3+ · Deno MIT. EJS is bundled with yt-dlp.")
            .into_any_element()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One recovery surface keeps diagnosis, status and consent together"
    )]
    pub(super) fn components_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.components.focus_pending {
            self.components.focus.focus(window, cx);
            self.components.focus_pending = false;
        }
        let theme = cx.theme();
        let busy = self.components.busy();
        let checking = self
            .components
            .reports
            .iter()
            .any(|r| r.state == ComponentState::Checking);
        let complete = self.components.progress == Progress::Complete && self.components.ready();
        let status = match &self.components.progress {
            Progress::Ready if busy => "Starting download…".into(),
            Progress::Ready if checking => "Checking your playback components…".into(),
            Progress::Ready => "Nothing downloads until you choose Download or Repair.".into(),
            Progress::Downloading { bytes, total } => format!(
                "Downloading: {}.{} / {}.{} MB",
                bytes / 1_000_000,
                bytes / 100_000 % 10,
                total / 1_000_000,
                total / 100_000 % 10
            ),
            Progress::Extracting => "Verifying and unpacking components…".into(),
            Progress::Checking => "Checking downloaded components…".into(),
            Progress::Complete if complete => "Playback components are ready.".into(),
            Progress::Complete if checking => {
                "Download complete. Checking active components…".into()
            }
            Progress::Complete => {
                "Downloaded components could not be activated. Check the details and try again."
                    .into()
            }
            Progress::Cancelled => "Download cancelled. Previous components were kept.".into(),
            Progress::Failed(reason) => reason.clone(),
        };
        let mut content = div().id("components-content").test_support().flex_1().min_h_0()
            .overflow_y_scroll().relative().track_scroll(&self.components.scroll).pr_3().vertical_scrollbar(&self.components.scroll).flex().flex_col().gap_4()
            .child(div().text_color(theme.muted_foreground).child(if complete {
                "You can return to your music. Playback won't start automatically."
            } else { "Grajek needs both components to play YouTube audio. Your queue and settings stay safe." }))
            .child(self.component_rows(cx));
        if system_override() {
            content = content.child(div().text_size(px(12.)).text_color(theme.danger)
                .child("GRAJEK_TOOLS=system is set. Remove it and restart Grajek to use downloads, or repair your system tools."));
        }
        if self.components.details {
            content = content.child(self.components_details(cx));
        }
        let mut feedback = div().flex().flex_col().gap_2().child(
            div()
                .id("components-progress")
                .test_support()
                .role(gpui_kit::Role::Status)
                .aria_label(status.clone())
                .line_clamp(2)
                .text_size(px(12.))
                .text_color(if matches!(self.components.progress, Progress::Failed(_)) {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(status),
        );
        if let Progress::Downloading { bytes, total } = self.components.progress {
            let percent = u8::try_from(
                bytes
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0)
                    .min(100),
            )
            .unwrap_or(0);
            feedback = feedback.child(
                gpui_kit::component::progress::Progress::new("components-download-progress")
                    .value(f32::from(percent))
                    .accessibility_label("Playback components download"),
            );
        }
        let mut actions = div()
            .id("components-actions")
            .test_support()
            .flex()
            .flex_col()
            .items_start()
            .gap_2();
        if !complete && !busy {
            actions = actions.child(
                self.button("components-download")
                    .primary()
                    .disabled(busy || checking || system_override())
                    .label(if matches!(self.components.progress, Progress::Failed(_)) {
                        "Try again"
                    } else {
                        self.components_action()
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.download_components(window, cx)),
                    ),
            );
        }
        actions = actions.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .when(busy, |row| {
                    row.child(
                        self.button("components-cancel")
                            .label("Cancel download")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(job) = &this.components.job {
                                    job.cancel();
                                }
                                cx.notify();
                            })),
                    )
                })
                .when(!busy && !checking && !complete, |row| {
                    row.child(
                        self.button("components-check")
                            .ghost()
                            .label("Check components")
                            .on_click(cx.listener(|this, _, _, cx| this.check_components(cx))),
                    )
                })
                .child(
                    self.button("components-details-toggle")
                        .ghost()
                        .label(if self.components.details {
                            "Hide details"
                        } else {
                            "Details"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.components.details = !this.components.details;
                            cx.notify();
                        })),
                )
                .child(
                    self.button("components-later")
                        .when(complete, ButtonVariants::primary)
                        .label(if busy {
                            "Keep downloading in background"
                        } else if complete {
                            "Back to music"
                        } else {
                            "Not now"
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.close_components(window, cx)),
                        ),
                ),
        );
        div()
            .id("components-dialog")
            .test_support()
            .role(gpui_kit::Role::Dialog)
            .aria_label("Playback components")
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_4()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(px(14.))
            .line_height(px(20.))
            .track_focus(&self.components.focus)
            .tab_stop(false)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_components(window, cx);
                }
            }))
            .child(
                div()
                    .text_size(px(20.))
                    .line_height(px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Playback components"),
            )
            .child(content)
            .child(feedback)
            .child(actions)
            .into_any_element()
    }
}

fn system_override() -> bool {
    std::env::var_os("GRAJEK_TOOLS").as_deref() == Some(std::ffi::OsStr::new("system"))
}
