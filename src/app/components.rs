use super::{
    ActiveTheme, ButtonVariants, Context, Disableable, FluentBuilder, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, PlayerApp,
    StatefulInteractiveElement, Styled, Task, TestSupportExt, Window, div, px,
};
use crate::media::components::{
    self, Need,
    download::{self, Download, Progress},
};
use gpui_kit::{AnyElement, FocusHandle};

pub(super) struct Panel {
    pub open: bool,
    focus_pending: bool,
    focus: FocusHandle,
    runtime: Option<tokio::runtime::Handle>,
    job: Option<Download>,
    progress: Progress,
    updates: Option<Task<()>>,
}

impl Panel {
    pub fn new(cx: &Context<PlayerApp>) -> Self {
        Self {
            open: false,
            focus_pending: false,
            focus: cx.focus_handle(),
            runtime: None,
            job: None,
            progress: Progress::Ready,
            updates: None,
        }
    }
}

impl PlayerApp {
    pub fn with_components(mut self, runtime: tokio::runtime::Handle) -> Self {
        self.components.runtime = Some(runtime);
        self.components.open = components::tools().need().is_some();
        self.components.focus_pending = self.components.open;
        self
    }

    pub(super) fn components_action(&self) -> &'static str {
        if self.components.job.is_some() {
            return "Download progress";
        }
        match components::tools().need() {
            Some(Need::Update) => "Update components",
            Some(Need::Repair) => "Re-download components",
            _ if components::tools().mode == components::Mode::Managed => "Re-download components",
            _ => "Download components",
        }
    }

    pub(super) fn open_components(&mut self, cx: &mut Context<Self>) {
        self.components.open = true;
        self.components.focus_pending = true;
        self.help.dismiss(cx);
        self.row_menu = None;
        self.queue_options_open = false;
        cx.notify();
    }

    fn close_components(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.components.open = false;
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

    #[expect(
        clippy::too_many_lines,
        reason = "One consent surface keeps the download disclosure, status and actions together"
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
        let pins = components::pins();
        let busy = self.components.job.is_some();
        let title = if components::tools().need() == Some(Need::Update) {
            "Update playback components"
        } else {
            "Playback components needed"
        };
        let mut panel = div()
            .id("components-dialog")
            .test_support()
            .role(gpui_kit::Role::Dialog)
            .aria_label(title)
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .overflow_y_scroll()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_sm()
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
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child("To play YouTube audio, Grajek needs these playback components:");
        if let Ok(pins) = pins {
            let megabytes = (pins.ytdlp.linux.size + pins.deno.linux.size).div_ceil(1_000_000);
            panel = panel.child(format!("yt-dlp {}: reads video and playlist information and the audio stream address. Includes yt-dlp-ejs {} for YouTube challenges.", pins.ytdlp.version, pins.ytdlp.ejs))
                .child(format!("Deno {}: JavaScript runtime used by yt-dlp.", pins.deno.version))
                .child(format!("Grajek can download them from their official GitHub releases (about {megabytes} MB), verify their checksums and keep them in your Grajek data directory. They won't be added to PATH. You can also install yt-dlp and Deno with your package manager."))
                .child("Licenses: yt-dlp GPLv3+ · Deno MIT")
                .child("Sources: https://github.com/yt-dlp/yt-dlp · https://github.com/denoland/deno");
        }
        if let Some(v) = components::tools().versions {
            panel = panel.child(format!("Installed: yt-dlp {}, EJS {}, Deno {}. You can keep these versions by choosing Later.", v.ytdlp, v.ejs, v.deno));
        }
        let status = match &self.components.progress {
            Progress::Ready if busy => "Starting download…".into(),
            Progress::Ready => "An internet connection is required. Nothing is downloaded until you choose Download.".into(),
            Progress::Downloading { bytes, total } => format!("Downloading: {bytes} / {total} bytes"),
            Progress::Extracting => "Verifying and unpacking playback components…".into(),
            Progress::Checking => "Checking playback component versions…".into(),
            Progress::Complete => "Playback components are ready. You can return to the player.".into(),
            Progress::Cancelled => "Download cancelled. Your previous playback components were kept.".into(),
            Progress::Failed(error) => error.clone(),
        };
        if let Progress::Downloading { bytes, total } = self.components.progress {
            let percent = u8::try_from(
                bytes
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0)
                    .min(100),
            )
            .unwrap_or(0);
            panel = panel.child(
                gpui_kit::component::progress::Progress::new("components-download-progress")
                    .value(f32::from(percent))
                    .accessibility_label("Playback components download"),
            );
        }
        panel
            .child(
                div()
                    .id("components-progress")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("components-download")
                            .primary()
                            .disabled(busy)
                            .label(if matches!(self.components.progress, Progress::Failed(_)) {
                                "Retry"
                            } else {
                                self.components_action()
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.download_components(window, cx);
                            })),
                    )
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
                    .child(
                        self.button("components-later")
                            .label(if busy {
                                "Keep downloading in background"
                            } else {
                                "Not now / Later"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_components(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
