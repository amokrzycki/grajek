mod controls;
mod history;
mod progress;
mod queue;

use gpui_kit::assets::IconName as AudioIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    slider::{SliderEvent, SliderState},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, TestSupportExt, Window, div, px,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::player::{Command, Phase, State, VolumeRequest};

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

pub struct PlayerApp {
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
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: mpsc::Sender<Command>,
        volume_requests: watch::Sender<VolumeRequest>,
        mut states: watch::Receiver<State>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste YouTube URL…"));
        window.focus(&input.read(cx).focus_handle(cx), cx);
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
        let app = Self {
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
            import_pending: false,
            dragging: false,
            volume_seq: 0,
            volume_pending: None,
            section: Section::Queue,
            confirm_clear: false,
            _subscriptions: subscriptions,
            _updates: updates,
        };
        app.sync_progress(window, cx);
        app
    }

    fn accept_state(&mut self, state: State, window: &mut Window, cx: &mut Context<Self>) {
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
        self.state = state;
        self.import_pending = false;
        self.pending = None;
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
        .child(text)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .test_support()
}

impl PlayerApp {
    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
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
            .justify_between()
            .gap_2()
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
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .track_focus(&self.voice_focus)
                    .tab_stop(false)
                    .text_xs()
                    .text_color(if self.state.available {
                        theme.muted_foreground
                    } else {
                        color
                    })
                    .child(div().size(px(6.)).flex_none().rounded_full().bg(color))
                    .child(label("voice-status", connection(&self.state).into()))
                    .child(self.leave_button(cx)),
            )
    }

    fn url_row(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap_1()
            .child(
                Input::new(&self.input)
                    .id("url")
                    .aria_label("YouTube video or playlist URL")
                    .w_full()
                    .h(px(32.))
                    .mb_2()
                    .bg(cx.theme().muted),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_1()
                    .child(self.repeat_button(cx))
                    .child(
                        Button::new("play")
                            .primary()
                            .label("Play URL")
                            .small()
                            .compact()
                            .h(px(28.))
                            .disabled(self.pending.is_some() || !self.state.can_play())
                            .on_click(cx.listener(|this, _, window, cx| this.play(window, cx))),
                    )
                    .child(
                        Button::new("enqueue")
                            .label("Add to queue")
                            .small()
                            .compact()
                            .h(px(28.))
                            .disabled(!self.state.can_enqueue())
                            .on_click(cx.listener(|this, _, _, cx| this.enqueue(cx))),
                    )
                    .child(self.import_button(cx)),
            )
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
            None => ("Nothing playing".into(), theme.muted_foreground),
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
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .py_2()
            .border_b_1()
            .border_color(crate::style::separator())
            .child(
                div()
                    .size(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(phase_color)
                    .bg(theme.muted)
                    .text_color(phase_color)
                    .child(Icon::new(symbol).size_5()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(title_color)
                            .child(label("title", title)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_xs()
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

    /// Errors wrap and clamp to three lines instead of stretching the window.
    fn error(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let import_visible = self.state.import != crate::player::ImportStatus::Ready;
        let seek_error = match &self.state.seek {
            crate::player::SeekStatus::Failed(message) => Some(message.clone()),
            _ => None,
        };
        let error = self.state.error.clone().or(seek_error);
        if error.is_none() && !import_visible {
            return None;
        }
        let import = import_visible.then(|| self.state.import.message());
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .children(import.map(|message| {
                    div()
                        .id("import-status")
                        .role(gpui_kit::Role::Status)
                        .aria_label(message.clone())
                        .flex()
                        .text_xs()
                        .text_color(
                            if matches!(self.state.import, crate::player::ImportStatus::Failed(_)) {
                                theme.danger
                            } else {
                                theme.muted_foreground
                            },
                        )
                        .child(label("import-message", message))
                        .test_support()
                }))
                .children(error.map(|error| {
                    let tip: SharedString = error.clone().into();
                    div()
                        .id("error")
                        .flex_none()
                        .w_full()
                        .px_2()
                        .py_1()
                        .rounded(theme.radius)
                        .bg(theme.button_danger)
                        .text_xs()
                        .text_color(theme.danger)
                        .line_clamp(if import_visible { 2 } else { 3 })
                        .child(error)
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .test_support()
                })),
        )
    }

    fn empty(id: &'static str, text: &'static str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .flex()
            .items_start()
            .px_2()
            .pt_3()
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
            .gap_2()
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
                            .child(Tab::new().label(format!("Queue ({})", self.state.queue.len())))
                            .child(Tab::new().label("History")),
                    )
                    .child(self.remember_button(cx)),
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
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
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
            .child(self.url_row(cx))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .child(self.now_playing(cx))
                    .child(self.progress_row(window, cx)),
            )
            .child(self.controls(cx))
            .children(self.error(cx))
            .child(self.volume_row(window, cx))
            .child(self.sections(cx))
    }
}

#[cfg(test)]
mod tests;
