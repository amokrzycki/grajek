use gpui_kit::component::{
    ActiveTheme, Disableable, button::ButtonVariants, scroll::ScrollableElement,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use crate::media::{
    Tool,
    components::{self, download::Progress, status::ComponentState, system_override},
};

use super::super::ui::escape;
use super::PlayerApp;
use super::update::{RemoteCheck, update_check_visible};

impl PlayerApp {
    pub(in crate::app) fn component_rows(&self, cx: &Context<Self>) -> AnyElement {
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

    fn component_update_row(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tools = components::tools();
        if !update_check_visible(tools.mode, tools.need(), system_override()) {
            return div()
                .id("components-update-status")
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child("System tools are updated outside Grajek.")
                .into_any_element();
        }
        let checking = matches!(self.components.remote, RemoteCheck::Checking { .. });
        let mut row = div().flex().flex_col().items_start().gap_1().child(
            self.focus
                .button("components-update-check")
                .ghost()
                .disabled(self.components.busy() || checking)
                .label("Check for component updates")
                .on_click(cx.listener(|this, _, _, cx| this.check_component_updates(cx))),
        );
        if let Some(status) = self.components.remote.status() {
            row = row.child(
                div()
                    .id("components-update-status")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label(status.clone())
                    .text_size(px(12.))
                    .text_color(
                        if matches!(self.components.remote, RemoteCheck::Failed(_)) {
                            theme.danger
                        } else {
                            theme.muted_foreground
                        },
                    )
                    .child(status),
            );
        }
        row.into_any_element()
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
        if let Ok(pins) = components::effective_pins() {
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
        if let Some(directory) = crate::paths::tools_dir() {
            details = details.child(format!("Download location: {}", directory.display()));
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
    pub(in crate::app) fn components_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.components.focus_pending {
            self.components.focus.focus(window, cx);
            self.components.focus_pending = false;
        }
        let update_row = self.component_update_row(cx);
        let theme = cx.theme();
        let busy = self.components.busy();
        let checking = self
            .components
            .reports
            .iter()
            .any(|r| r.state == ComponentState::Checking);
        let complete = self.components.progress == Progress::Complete
            && self.components.ready()
            && !matches!(self.components.remote, RemoteCheck::Available { .. });
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
        let mut content = div()
            .id("components-content")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .relative()
            .track_scroll(&self.components.scroll)
            .pr_3()
            .vertical_scrollbar(&self.components.scroll)
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(if complete {
                        "You can return to your music. Playback won't start automatically."
                    } else {
                        "Grajek needs both components to play YouTube audio. Your queue and settings stay safe."
                    }),
            )
            .child(self.component_rows(cx))
            .child(update_row);
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
                self.focus
                    .button("components-download")
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
                        self.focus
                            .button("components-cancel")
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
                        self.focus
                            .button("components-check")
                            .ghost()
                            .label("Check components")
                            .on_click(cx.listener(|this, _, _, cx| this.check_components(cx))),
                    )
                })
                .child(
                    self.focus
                        .button("components-details-toggle")
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
                    self.focus
                        .button("components-later")
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
                if escape(&event.keystroke.key) {
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
