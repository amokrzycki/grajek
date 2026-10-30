mod controls;
mod history;
mod lists;
mod progress;
mod queue;
mod settings;

use gpui_kit::assets::IconName as AudioIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    popover::Popover,
    slider::{SliderEvent, SliderState},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, TestSupportExt, Window, div, px, relative,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::player::{Command, Phase, State, VolumeRequest};
use crate::style::space;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Queue,
    History,
}

#[derive(Clone, Copy)]
enum Pending {
    Play,
    Next,
    Pause,
    Stop,
    Leave,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent UI focus, pending-operation and panel flags are not a single state machine"
)]
pub struct PlayerApp {
    token: Entity<InputState>,
    settings_open: bool,
    url_actions_open: bool,
    queue_options_open: bool,
    setup_starting: bool,
    setup_help: bool,
    input: Entity<InputState>,
    volume: Entity<SliderState>,
    volume_focus: FocusHandle,
    progress: Entity<SliderState>,
    progress_focus: FocusHandle,
    progress_gesture: Option<u64>,
    progress_preview: Option<Duration>,
    /// Tracks the Pause/Next/Stop row, to rescue keyboard focus from a control that turns off.
    controls_focus: FocusHandle,
    voice_focus: FocusHandle,
    import_focus: FocusHandle,
    commands: mpsc::Sender<Command>,
    volume_requests: watch::Sender<VolumeRequest>,
    state: State,
    /// Prevent duplicate input while keeping the authoritative playback snapshot visible.
    pending: Option<Pending>,
    transport_seq: u64,
    queue_scroll: lists::ListNavigation,
    history_scroll: lists::ListNavigation,
    import_pending: bool,
    /// The slider is being dragged; controller snapshots must not fight the pointer.
    dragging: bool,
    volume_seq: u64,
    /// The newest volume request the controller has not yet answered.
    volume_pending: Option<u64>,
    section: Section,
    /// "Clear history" was pressed once and awaits confirmation.
    confirm_clear: bool,
    _subscriptions: Vec<Subscription>,
    _updates: Task<()>,
    #[cfg(target_os = "windows")]
    media: Option<crate::native::windows::MediaSession>,
}

fn clock(position: Duration) -> String {
    let seconds = position.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// `01:24 / 03:45`, or `01:24 (duration unavailable)` when yt-dlp gave no duration.
fn format_progress(position: Duration, duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("{} / {}", clock(position), clock(duration)),
        None => format!("{} (duration unavailable)", clock(position)),
    }
}

impl PlayerApp {
    #[cfg(target_os = "windows")]
    pub fn with_media(mut self, media: Option<crate::native::windows::MediaSession>) -> Self {
        self.media = media;
        self
    }
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: mpsc::Sender<Command>,
        volume_requests: watch::Sender<VolumeRequest>,
        mut states: watch::Receiver<State>,
    ) -> Self {
        lists::init(cx);
        controls::init(cx);
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Bot token")
        });
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste YouTube link"));
        window.focus(&token.read(cx).focus_handle(cx), cx);
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(f32::from(volume_requests.borrow().percent))
        });
        let progress = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1000.)
                .step(1.)
                .default_value(0.)
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { secondary, .. } = event {
                    // Enter plays; Ctrl+Enter queues.
                    if *secondary {
                        this.enqueue(cx);
                    } else {
                        this.play(window, cx);
                    }
                }
            }),
            cx.subscribe_in(&volume, window, |this, _, event, _, cx| match event {
                SliderEvent::Change(value) => {
                    this.dragging = true;
                    this.request_volume(value.end(), false, cx);
                }
                SliderEvent::Release(value) => {
                    this.dragging = false;
                    this.request_volume(value.end(), true, cx);
                }
            }),
            cx.subscribe_in(&progress, window, |this, _, event, window, cx| {
                this.progress_event(event, window, cx);
            }),
        ];
        let state = states.borrow_and_update().clone();
        let updates = cx.spawn_in(window, async move |entity, cx| {
            while states.changed().await.is_ok() {
                let state = states.borrow_and_update().clone();
                if entity
                    .update_in(cx, |this, window, cx| {
                        this.accept_state(state, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let transport_seq = state.transport_ack;
        let mut app = Self {
            token,
            settings_open: true,
            url_actions_open: false,
            queue_options_open: false,
            setup_starting: true,
            setup_help: false,
            input,
            volume,
            volume_focus: cx.focus_handle(),
            progress,
            progress_focus: cx.focus_handle(),
            progress_gesture: None,
            progress_preview: None,
            controls_focus: cx.focus_handle(),
            voice_focus: cx.focus_handle(),
            import_focus: cx.focus_handle(),
            commands,
            volume_requests,
            state,
            pending: None,
            transport_seq,
            queue_scroll: lists::ListNavigation::new(),
            history_scroll: lists::ListNavigation::new(),
            import_pending: false,
            dragging: false,
            volume_seq: 0,
            volume_pending: None,
            section: Section::Queue,
            confirm_clear: false,
            _subscriptions: subscriptions,
            _updates: updates,
            #[cfg(target_os = "windows")]
            media: None,
        };
        app.sync_lists(cx);
        app.sync_progress(window, cx);
        app
    }

    fn accept_state(&mut self, state: State, window: &mut Window, cx: &mut Context<Self>) {
        if self.setup_starting && state.controller_ready && !state.setup_busy {
            if state.setup.destination.is_some() {
                self.settings_open = false;
                window.focus(&self.input.read(cx).focus_handle(cx), cx);
            }
            self.setup_starting = false;
        }
        let before = self.controls_enabled();
        let lost_gateway = self.state.available && !state.available;
        let lost_voice_control = self.state.can_leave_voice()
            && !state.can_leave_voice()
            && self.voice_focus.contains_focused(window, cx);
        let import_finished = self.state.import == crate::player::ImportStatus::Importing
            && state.import != crate::player::ImportStatus::Importing
            && self.import_focus.contains_focused(window, cx);
        let lost_progress = self.state.can_seek()
            && (!state.can_seek() || self.state.track_id != state.track_id)
            && self.progress_focus.is_focused(window);
        if self.state.track_id != state.track_id || !state.can_seek() {
            self.progress_preview = None;
        }
        if lost_gateway {
            self.queue_options_open = false;
            self.url_actions_open = false;
        }
        self.state = state;
        self.import_pending = false;
        if self.state.transport_ack >= self.transport_seq {
            self.pending = None;
        }
        self.sync_lists(cx);
        // Gateway loss also disables preferences outside the transport group.
        if lost_gateway || lost_voice_control || import_finished || lost_progress {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        } else {
            self.rescue_focus(before, window, cx);
        }
        self.sync_volume(window, cx);
        self.sync_progress(window, cx);
        cx.notify();
    }

    /// Fire-and-forget: the controller answers every command with a new snapshot, and the
    /// window only ever renders snapshots.
    fn send(&self, command: Command) -> bool {
        self.commands.try_send(command).is_ok()
    }

    fn input_text(&self, cx: &Context<Self>) -> String {
        self.input.read(cx).value().to_string()
    }

    fn select_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.queue_options_open = false;
        self.confirm_clear = false;
        cx.notify();
    }
}

fn connection(state: &State) -> &'static str {
    if state.phase == Phase::Disconnecting {
        "Leaving Discord voice…"
    } else if state.available && state.voice_connected {
        "Discord voice connected"
    } else if state.available {
        "Discord ready · voice idle"
    } else if state.phase == Phase::Initializing {
        "Connecting to Discord…"
    } else {
        "Discord unavailable"
    }
}

/// A one-line, ellipsized label whose full text appears in a tooltip.
fn label(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    let tip = text.clone();
    label_with_tooltip(id, text, tip)
}

fn label_with_tooltip(id: impl Into<ElementId>, text: String, tip: String) -> impl IntoElement {
    let tip: SharedString = tip.into();
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .truncate()
        .aria_label(text.clone())
        .child(text)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .test_support()
}

impl PlayerApp {
    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = connection(&self.state);
        let (settings_icon, settings_label) = if self.settings_open {
            (AudioIcon::ArrowLeft, "Back to player")
        } else {
            (AudioIcon::Settings, "Discord settings")
        };
        let color = if self.state.available {
            theme.green
        } else if self.state.phase == Phase::Initializing {
            theme.muted_foreground
        } else {
            theme.danger
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Grajek"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .line_height(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(5.)).flex_none().rounded_full().bg(color))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(if self.state.available {
                                        theme.muted_foreground
                                    } else {
                                        color
                                    })
                                    .child(label("voice-status", status.into())),
                            ),
                    )
                    .children(self.state.setup.destination.as_ref().map(|destination| {
                        div()
                            .flex()
                            .min_w_0()
                            .text_color(theme.muted_foreground)
                            .child(label(
                                "voice-destination",
                                format!(
                                    "{} / {}",
                                    destination.guild_name, destination.channel_name
                                ),
                            ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .track_focus(&self.voice_focus)
                    .tab_stop(false)
                    .child(self.leave_button(cx))
                    .child(
                        Button::new("settings")
                            .icon(settings_icon)
                            .accessibility_label(settings_label)
                            .tooltip(settings_label)
                            .ghost()
                            .flex_none()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings_open = !this.settings_open;
                                this.queue_options_open = false;
                                this.url_actions_open = false;
                                if this.settings_open {
                                    window.focus(&this.token.read(cx).focus_handle(cx), cx);
                                } else {
                                    window.focus(&this.input.read(cx).focus_handle(cx), cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    fn url_row(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let importing = self.state.import == crate::player::ImportStatus::Importing;
        div().flex().flex_none().flex_col().gap(space::GROUP)
            .child({
                let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
                let theme = cx.theme();
                // One frame owns the boundary; kit Input exposes no hover styling API.
                div().id("url").w_full().h(px(32.)).rounded(theme.radius)
                    .border_1().border_color(if focused { theme.ring } else { theme.input })
                    .hover(move |frame| frame.border_color(if focused { theme.ring } else { theme.muted_foreground }))
                    .child(Input::new(&self.input).id("url-editor")
                        .aria_label("Paste YouTube link").bordered(false)
                        .w_full().h_full().bg(theme.muted))
                    .test_support()
            })
            .child(div().flex().items_center().gap_2()
                .child(Button::new("play")
                    .when(!matches!(self.state.phase, Phase::Playing | Phase::Paused), ButtonVariants::primary)
                    .label("Play now")
                    .tooltip("Play a single video · Enter. A watch link with a playlist still plays only its video.")
                    .disabled(!self.play_enabled())
                    .on_click(cx.listener(|this, _, window, cx| this.play(window, cx))))
                .child(Button::new("enqueue").label("Add to queue")
                    .disabled(!self.state.can_enqueue())
                    .tooltip(if self.state.queue.len() >= crate::queue::QUEUE_LIMIT { "Queue full · remove a track to add another" } else { "Add a single video · Ctrl+Enter" })
                    .on_click(cx.listener(|this, _, _, cx| this.enqueue(cx))))
                .child(Popover::new("url-actions-popover")
                    .open(self.url_actions_open)
                    .on_open_change(cx.listener(|this, open, _, cx| { this.url_actions_open = *open; cx.notify(); }))
                    .anchor(gpui_kit::Anchor::TopRight)
                    .trigger(Button::new("url-more").label("More").dropdown_caret(true)
                        .ghost().accessibility_label("More link actions")
                        .tooltip("Import a public YouTube playlist"))
                    .child(div().flex().flex_col().w(px(240.)).gap_2()
                        .when(!importing, |menu| menu.child(self.import_button(cx)))
                        .child(div().px_2().text_xs().text_color(cx.theme().muted_foreground)
                            .child("Uses the link above. Adds up to 100 tracks without interrupting playback.")))))
            .children((self.state.import != crate::player::ImportStatus::Ready).then(|| {
                div().flex().items_center().gap_2()
                    .child(div().id("import-status").role(gpui_kit::Role::Status)
                        .aria_label(self.state.import.message()).flex().flex_1().min_w_0().text_xs()
                        .text_color(if matches!(self.state.import, crate::player::ImportStatus::Failed(_)) { cx.theme().danger } else { cx.theme().muted_foreground })
                        .child(label("import-message", self.state.import.message())).test_support())
                    .when(importing, |row| row.child(self.import_button(cx)))
            }))
    }

    /// Title and status are single lines that truncate with an ellipsis, so a long
    /// title can never widen the window or push the controls away.
    fn now_playing(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let phase_color = if self.state.error.is_some() {
            theme.danger
        } else if self.state.phase == Phase::Playing {
            theme.primary
        } else if self.state.phase == Phase::Paused {
            theme.yellow
        } else {
            theme.muted_foreground
        };
        let track = self.state.visible_track();
        let (title, title_color) = match track {
            Some(info) => (
                info.title
                    .clone()
                    .unwrap_or_else(|| "Untitled track".into()),
                theme.foreground,
            ),
            None => (
                match self.state.phase {
                    Phase::Ready | Phase::Error if !self.state.available => "Connect to Discord",
                    Phase::Ready => "Ready for a link",
                    Phase::Initializing => "Starting Grajek…",
                    Phase::Connecting => "Joining voice…",
                    Phase::Resolving => "Preparing your track…",
                    Phase::Stopping => "Stopping playback…",
                    Phase::Disconnecting => "Leaving voice…",
                    Phase::Error => "Playback stopped",
                    Phase::Playing | Phase::Paused => "Untitled track",
                }
                .into(),
                theme.muted_foreground,
            ),
        };
        let progress = track.map(|info| format_progress(self.state.position, info.duration));
        let symbol = match self.state.phase {
            Phase::Playing => AudioIcon::Play,
            Phase::Paused => AudioIcon::Pause,
            Phase::Ready | Phase::Error => AudioIcon::Square,
            Phase::Initializing
            | Phase::Connecting
            | Phase::Resolving
            | Phase::Stopping
            | Phase::Disconnecting => AudioIcon::Ellipsis,
        };
        div().flex().flex_none().items_center().child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .text_size(px(24.))
                        .line_height(relative(1.25))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(title_color)
                        .child(label("title", title)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .line_height(px(16.))
                        .child(Icon::new(symbol).size(px(12.)).text_color(phase_color))
                        .child(
                            div()
                                .id("seek-status")
                                .flex_none()
                                .role(gpui_kit::Role::Status)
                                .text_color(phase_color)
                                .when(self.progress_preview.is_some(), |status| {
                                    status.font_family(theme.mono_font_family.clone())
                                })
                                .child(self.progress_status())
                                .test_support(),
                        )
                        .children(progress.map(|progress| {
                            div()
                                .id("progress")
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.muted_foreground)
                                .child(progress)
                                .test_support()
                        })),
                ),
        )
    }

    /// Keep failures readable without displacing the scrolling list.
    fn error(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let seek_error = match &self.state.seek {
            crate::player::SeekStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let error = self.state.error.clone().or(seek_error)?;
        let tip: SharedString = error.clone().into();
        Some(
            div()
                .id("error")
                .role(gpui_kit::Role::Alert)
                .flex_none()
                .w_full()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .bg(theme.button_danger)
                .text_xs()
                .line_height(px(16.))
                .text_color(theme.danger)
                .line_clamp(2)
                .child(error)
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .test_support(),
        )
    }

    fn empty(id: &'static str, text: &'static str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .flex()
            .items_start()
            .pt_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .test_support()
    }

    fn sections(&self, cx: &Context<Self>) -> impl IntoElement {
        let selected = match self.section {
            Section::Queue => 0,
            Section::History => 1,
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        TabBar::new("sections")
                            .underline()
                            .small()
                            .selected_index(selected)
                            .on_click(cx.listener(|this, index: &usize, _, cx| {
                                this.select_section(
                                    if *index == 0 {
                                        Section::Queue
                                    } else {
                                        Section::History
                                    },
                                    cx,
                                );
                            }))
                            .child(Tab::new().label(format!(
                                "Up next · {}{}",
                                self.state.queue.len(),
                                if self.state.queue.len() >= crate::queue::QUEUE_LIMIT {
                                    " · Full"
                                } else {
                                    ""
                                }
                            )))
                            .child(Tab::new().label("History")),
                    )
                    .child(self.queue_options(cx)),
            )
            .child(match self.section {
                Section::Queue => self.queue_list(cx).into_any_element(),
                Section::History => self.history_list(cx).into_any_element(),
            })
    }
}

impl Render for PlayerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        if self.settings_open {
            return div()
                .size_full()
                .bg(theme.background)
                .text_color(theme.foreground)
                .font_family(theme.font_family.clone())
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.header(cx))
                .child(self.settings(cx))
                .into_any_element();
        }
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p(space::INSET)
            .text_sm()
            .flex()
            .flex_col()
            .gap(if window.viewport_size().height <= px(480.) {
                space::GROUP
            } else {
                space::SECTION
            })
            .overflow_hidden()
            // The kit's tabs cannot take focus, so Ctrl+1 / Ctrl+2 switch sections.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.control {
                    match event.keystroke.key.as_str() {
                        "1" => this.select_section(Section::Queue, cx),
                        "2" => this.select_section(Section::History, cx),
                        _ => {}
                    }
                }
            }))
            .child(self.header(cx))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(space::TIGHT)
                    .child(self.now_playing(cx))
                    .children(self.progress_row(window, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(space::DETAIL)
                            .child(self.controls(cx))
                            .child(
                                div()
                                    .w(px(120.))
                                    .flex_none()
                                    .child(self.volume_row(window, cx)),
                            ),
                    )
                    .children(self.error(cx)),
            )
            .child(self.url_row(window, cx))
            .child(self.sections(cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
