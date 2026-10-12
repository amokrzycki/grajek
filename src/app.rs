use gpui_kit::assets::IconName as SpeakerIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    slider::{Slider, SliderEvent, SliderState},
    tab::{Tab, TabBar},
    tooltip::Tooltip,
};
use gpui_kit::{
    AppContext, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, TestSupportExt, Window, div, prelude::FluentBuilder,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::{
    player::{Command, Phase, State, VolumeRequest, percent_from},
    queue::{QUEUE_LIMIT, QueueItem},
    store::HistoryEntry,
};

/// Volume change per arrow key; Page Up/Down move twice as far.
const VOLUME_STEP: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Queue,
    History,
}

pub struct PlayerApp {
    input: Entity<InputState>,
    volume: Entity<SliderState>,
    volume_focus: FocusHandle,
    /// Tracks the Pause/Next/Stop row, to rescue keyboard focus from a control that turns off.
    controls_focus: FocusHandle,
    commands: mpsc::Sender<Command>,
    volume_requests: watch::Sender<VolumeRequest>,
    state: State,
    /// A Pause/Resume/Next request is in flight; cleared by the controller's next state.
    control_pending: bool,
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
        ];
        let state = states.borrow_and_update().clone();
        let updates = cx.spawn_in(window, async move |entity, cx| {
            while states.changed().await.is_ok() {
                let state = states.borrow_and_update().clone();
                if entity
                    .update_in(cx, |this, window, cx| {
                        let before = this.controls_enabled();
                        this.state = state;
                        this.control_pending = false;
                        this.rescue_focus(before, window, cx);
                        this.sync_volume(window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            input,
            volume,
            volume_focus: cx.focus_handle(),
            controls_focus: cx.focus_handle(),
            commands,
            volume_requests,
            state,
            control_pending: false,
            dragging: false,
            volume_seq: 0,
            volume_pending: None,
            section: Section::Queue,
            confirm_clear: false,
            _subscriptions: subscriptions,
            _updates: updates,
        }
    }

    /// Fire-and-forget: the controller answers every command with a new snapshot, and the
    /// window only ever renders snapshots.
    fn send(&self, command: Command) -> bool {
        self.commands.try_send(command).is_ok()
    }

    fn input_text(&self, cx: &Context<Self>) -> String {
        self.input.read(cx).value().to_string()
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.input_text(cx);
        self.play_url(url, window, cx);
    }

    fn play_url(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.can_play() {
            return;
        }
        let before = self.controls_enabled();
        if self.send(Command::Play(url)) {
            // Close the gap before the controller's first update arrives.
            self.state.phase = Phase::Connecting;
            self.state.error = None;
            self.state.track = None;
            self.control_pending = false;
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    fn enqueue(&self, cx: &Context<Self>) {
        let url = self.input_text(cx);
        self.enqueue_url(url);
    }

    fn enqueue_url(&self, url: String) {
        if self.state.can_enqueue() {
            self.send(Command::Enqueue(url));
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.control_pending || !self.state.can_next() {
            return;
        }
        let before = self.controls_enabled();
        if self.send(Command::Next) {
            self.state.phase = Phase::Connecting;
            self.state.error = None;
            self.state.track = None;
            // Disabled until the controller answers, so a double click skips one track.
            self.control_pending = true;
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.controls_enabled();
        if self.state.can_stop() && self.send(Command::Stop) {
            self.state.phase = Phase::Stopping;
            self.state.track = None;
            self.control_pending = false;
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    /// Sends Pause or Resume without guessing the outcome: the button keeps its label
    /// (and is disabled against repeat clicks) until the controller reports the real state.
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.control_pending {
            return;
        }
        let command = if self.state.can_pause() {
            Command::Pause
        } else if self.state.can_resume() {
            Command::Resume
        } else {
            return;
        };
        if self.send(command) {
            self.control_pending = true;
            cx.notify();
        }
    }

    /// Hands the newest slider value to the controller. The `watch` channel keeps only the
    /// latest one, so dragging never queues a stream of commands.
    fn request_volume(&mut self, value: f32, commit: bool, cx: &mut Context<Self>) {
        let Some(percent) = percent_from(value) else {
            return;
        };
        self.volume_seq += 1;
        self.volume_pending = Some(self.volume_seq);
        self.volume_requests.send_replace(VolumeRequest {
            percent,
            seq: self.volume_seq,
            commit,
        });
        cx.notify();
    }

    fn slider_value(&self, cx: &Context<Self>) -> f32 {
        self.volume.read(cx).value().end()
    }

    /// Keeps the slider equal to the volume the controller really applied. Until it has
    /// answered the newest request (or while dragging), the user's own value stands.
    fn sync_volume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            return;
        }
        if let Some(seq) = self.volume_pending {
            if self.state.volume_seq < seq {
                return;
            }
            self.volume_pending = None;
        }
        let applied = f32::from(self.state.volume);
        if (self.slider_value(cx) - applied).abs() > f32::EPSILON {
            self.volume
                .update(cx, |slider, cx| slider.set_value(applied, window, cx));
        }
    }

    /// The slider has no keyboard handling of its own: arrows, Page Up/Down, Home and End.
    fn volume_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.available {
            return;
        }
        let current = self.slider_value(cx);
        let target = match key {
            "left" | "down" => current - VOLUME_STEP,
            "right" | "up" => current + VOLUME_STEP,
            "pagedown" => current - 2.0 * VOLUME_STEP,
            "pageup" => current + 2.0 * VOLUME_STEP,
            "home" => 0.0,
            "end" => 100.0,
            _ => return,
        };
        let target = target.clamp(0.0, 100.0);
        self.volume
            .update(cx, |slider, cx| slider.set_value(target, window, cx));
        self.request_volume(target, true, cx);
    }

    /// Pause/Resume, Next and Stop as the controller currently allows them.
    fn controls_enabled(&self) -> [bool; 3] {
        [
            self.state.can_pause() || self.state.can_resume(),
            self.state.can_next(),
            self.state.can_stop(),
        ]
    }

    /// A focused button that becomes disabled would keep focus and block Tab, so focus
    /// moves back to the URL field instead.
    fn rescue_focus(&self, before: [bool; 3], window: &mut Window, cx: &mut Context<Self>) {
        let lost = before
            .iter()
            .zip(self.controls_enabled())
            .any(|(was, now)| *was && !now);
        if lost && self.controls_focus.contains_focused(window, cx) {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    fn select_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.confirm_clear = false;
        cx.notify();
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.confirm_clear {
            self.send(Command::ClearHistory);
        }
        self.confirm_clear = !self.confirm_clear;
        cx.notify();
    }
}

fn connection(state: &State) -> &'static str {
    if state.available {
        "Discord connected"
    } else if state.phase == Phase::Initializing {
        "Connecting to Discord…"
    } else {
        "Discord unavailable"
    }
}

/// A one-line, ellipsized label whose full text appears in a tooltip.
fn label(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    let tip: SharedString = text.clone().into();
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
            .items_center()
            .gap_1()
            .text_xs()
            .text_color(color)
            .child("●")
            .child(connection(&self.state))
    }

    fn url_row(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .gap_2()
            .child(Input::new(&self.input).id("url").flex_1().min_w_0())
            .child(
                Button::new("play")
                    .primary()
                    .label("Play URL")
                    .disabled(!self.state.can_play())
                    .on_click(cx.listener(|this, _, window, cx| this.play(window, cx))),
            )
            .child(
                Button::new("enqueue")
                    .label("Add to queue")
                    .disabled(!self.state.can_enqueue())
                    .on_click(cx.listener(|this, _, _, cx| this.enqueue(cx))),
            )
    }

    /// Title and status are single lines that truncate with an ellipsis, so a long
    /// title can never widen the window or push the controls away.
    fn now_playing(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let phase_color = if self.state.error.is_some() {
            theme.danger
        } else if self.state.phase == Phase::Playing {
            theme.green
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
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Now playing"),
            )
            .child(
                div()
                    .flex()
                    .text_base()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(title_color)
                    .child(label("title", title)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_none()
                            .text_color(phase_color)
                            .child("●")
                            .child(self.state.phase.label()),
                    )
                    .children(progress.map(|progress| {
                        div()
                            .id("progress")
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(progress)
                            .test_support()
                    })),
            )
    }

    fn controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let resume = self.state.phase == Phase::Paused;
        let toggle_enabled =
            !self.control_pending && (self.state.can_pause() || self.state.can_resume());
        let next_enabled = !self.control_pending && self.state.can_next();
        div()
            .flex()
            .gap_2()
            .track_focus(&self.controls_focus)
            .tab_stop(false)
            .child(
                Button::new("pause-resume")
                    .label(if resume { "Resume" } else { "Pause" })
                    .flex_1()
                    .disabled(!toggle_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(
                Button::new("next")
                    .label("Next")
                    .w_24()
                    .tooltip(if self.state.queue.is_empty() {
                        "Nothing is queued"
                    } else {
                        "Skip to the next queued track"
                    })
                    .disabled(!next_enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
            )
            .child(
                Button::new("stop")
                    .label("Stop")
                    .w_24()
                    .disabled(!self.state.can_stop())
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
    }

    /// Errors wrap and clamp to three lines instead of stretching the window.
    fn error(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let theme = cx.theme();
        self.state.error.clone().map(|error| {
            div()
                .id("error")
                .w_full()
                .text_xs()
                .text_color(theme.danger)
                .line_clamp(3)
                .child(error)
                .test_support()
        })
    }

    fn volume_row(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let available = self.state.available;
        let percent = percent_from(self.slider_value(cx)).unwrap_or(self.state.volume);
        let focused = self.volume_focus.is_focused(window);
        let speaker = if percent == 0 {
            SpeakerIcon::VolumeX
        } else {
            SpeakerIcon::Volume2
        };
        // A tracked handle decides its own tab order: reachable by Tab only while usable.
        let focus = self.volume_focus.clone().tab_stop(available);
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(speaker)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .id("volume")
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused {
                        theme.ring
                    } else {
                        theme.transparent
                    })
                    .track_focus(&self.volume_focus)
                    .capture_any_mouse_down(move |_, window, cx| window.focus(&focus, cx))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.volume_key(event.keystroke.key.as_str(), window, cx);
                    }))
                    .child(Slider::new(&self.volume).disabled(!available)),
            )
            .child(
                div()
                    .id("volume-percent")
                    .w_10()
                    .flex_none()
                    .text_right()
                    .text_sm()
                    .text_color(if available {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(format!("{percent}%"))
                    .test_support(),
            )
    }

    fn queue_row(
        position: usize,
        last: bool,
        item: &QueueItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let id = item.id;
        div()
            .id(("queue-row", id))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h_8()
            .rounded_md()
            .text_sm()
            .hover(|row| row.bg(theme.list_hover))
            .child(
                div()
                    .w_5()
                    .flex_none()
                    .text_right()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}", position + 1)),
            )
            .child(label(("queue-title", id), item.label().to_owned()))
            .child(
                Button::new(("queue-up", id))
                    .icon(IconName::ArrowUp)
                    .ghost()
                    .xsmall()
                    .tooltip("Move up")
                    .disabled(position == 0)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: true });
                    })),
            )
            .child(
                Button::new(("queue-down", id))
                    .icon(IconName::ArrowDown)
                    .ghost()
                    .xsmall()
                    .tooltip("Move down")
                    .disabled(last)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: false });
                    })),
            )
            .child(
                Button::new(("queue-remove", id))
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip("Remove from queue")
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Dequeue(id));
                    })),
            )
            .test_support()
    }

    fn history_row(
        &self,
        index: usize,
        entry: &HistoryEntry,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let title = entry
            .title
            .clone()
            .unwrap_or_else(|| crate::queue::video_id(&entry.url).to_owned());
        let url = entry.url.clone();
        let (play_url, queue_url, forget_url) = (url.clone(), url.clone(), url);
        div()
            .id(("history-row", index))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h_8()
            .rounded_md()
            .text_sm()
            .hover(|row| row.bg(theme.list_hover))
            .child(label(("history-title", index), title))
            .children(entry.duration().map(|duration| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(clock(duration))
            }))
            .child(
                Button::new(("history-play", index))
                    .icon(IconName::Play)
                    .ghost()
                    .xsmall()
                    .tooltip("Play now")
                    .disabled(!self.state.can_play())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.play_url(play_url.clone(), window, cx);
                    })),
            )
            .child(
                Button::new(("history-queue", index))
                    .icon(IconName::Plus)
                    .ghost()
                    .xsmall()
                    .tooltip("Add to queue")
                    .disabled(!self.state.can_enqueue())
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.enqueue_url(queue_url.clone());
                    })),
            )
            .child(
                Button::new(("history-remove", index))
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip("Remove from history")
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::ForgetHistory(forget_url.clone()));
                    })),
            )
            .test_support()
    }

    fn empty(id: &'static str, text: &'static str, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id(id)
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .text_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .test_support()
    }

    fn queue_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.state.queue.len();
        let last = count.saturating_sub(1);
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
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id("queue-count")
                            .child(format!(
                                "{count} pending{}",
                                if count >= QUEUE_LIMIT { " (full)" } else { "" }
                            ))
                            .test_support(),
                    )
                    .child(
                        Button::new("queue-clear")
                            .label("Clear queue")
                            .ghost()
                            .xsmall()
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, _, _| {
                                this.send(Command::ClearQueue);
                            })),
                    ),
            )
            .child(if count == 0 {
                Self::empty(
                    "queue-empty",
                    "The queue is empty. Paste a link and press Add to queue.",
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .id("queue-list")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scrollbar()
                    .children(self.state.queue.iter().enumerate().map(|(position, item)| {
                        Self::queue_row(position, position == last, item, cx)
                    }))
                    .into_any_element()
            })
    }

    fn history_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let count = self.state.history.len();
        let confirming = self.confirm_clear && count > 0;
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
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .id("history-count")
                            .child(format!("{count} saved"))
                            .test_support(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .when(confirming, |row| {
                                row.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .text_color(theme.foreground)
                                        .child(format!("Clear all {count}?")),
                                )
                            })
                            .child(
                                Button::new("history-clear")
                                    .label(if confirming {
                                        "Yes, clear"
                                    } else {
                                        "Clear history"
                                    })
                                    .when(confirming, ButtonVariants::danger)
                                    .when(!confirming, ButtonVariants::ghost)
                                    .xsmall()
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                            )
                            .when(confirming, |row| {
                                row.child(
                                    Button::new("history-clear-cancel")
                                        .label("Cancel")
                                        .ghost()
                                        .xsmall()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_clear = false;
                                            cx.notify();
                                        })),
                                )
                            }),
                    ),
            )
            .children(self.state.notice.clone().map(|notice| {
                div()
                    .id("notice")
                    .text_xs()
                    .text_color(theme.yellow)
                    .line_clamp(2)
                    .child(notice)
                    .test_support()
            }))
            .child(if count == 0 {
                Self::empty(
                    "history-empty",
                    "Nothing played yet. Videos appear here once they start playing.",
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .id("history-list")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scrollbar()
                    .children(
                        self.state
                            .history
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| self.history_row(index, entry, cx)),
                    )
                    .into_any_element()
            })
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
                TabBar::new("sections")
                    .underline()
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
            .gap_3()
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
            .child(self.now_playing(cx))
            .child(self.controls(cx))
            .children(self.error(cx))
            .child(self.volume_row(window, cx))
            .child(self.sections(cx))
    }
}
#[cfg(test)]
mod tests {
    use super::{Command, Phase, PlayerApp, Section, State, VolumeRequest, clock, format_progress};
    use crate::{media::TrackInfo, queue::QueueItem, store::HistoryEntry};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, ClipboardItem, ElementId, Entity, Focusable,
        ScrollDelta, TestAppContext, WindowBounds, WindowOptions, point, px, size,
    };
    use std::time::Duration;
    use tokio::sync::{mpsc, watch};

    fn state(phase: Phase) -> State {
        State {
            phase,
            available: true,
            ..State::default()
        }
    }

    fn playing(phase: Phase, title: Option<&str>, seconds: u64) -> State {
        State {
            track: Some(TrackInfo {
                title: title.map(Into::into),
                duration: Some(Duration::from_secs(225)),
            }),
            position: Duration::from_secs(seconds),
            ..state(phase)
        }
    }

    type Harness = (
        AnyWindowHandle,
        Entity<PlayerApp>,
        mpsc::Receiver<Command>,
        watch::Sender<State>,
    );

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn open_full(
        cx: &mut TestAppContext,
        initial: State,
    ) -> (Harness, watch::Receiver<VolumeRequest>) {
        let (commands, received) = mpsc::channel(16);
        let (volume, volume_rx) = watch::channel(VolumeRequest::default());
        let (states, updates) = watch::channel(initial);
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(520.), px(560.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| PlayerApp::new(window, cx, commands, volume, updates)),
            )
            .expect("test window should open")
        });
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("test window should render");
        ((window, view, received, states), volume_rx)
    }

    fn open(cx: &mut TestAppContext, initial: State) -> Harness {
        open_full(cx, initial).0
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .expect("test window should dispatch the click");
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn button(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        id: impl Into<ElementId>,
    ) -> (String, bool) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let snapshot = window.find(id);
            (
                snapshot.label().unwrap_or_default().to_owned(),
                // GPUI Kit's Button reports no `disabled` flag; a disabled button is not focusable.
                snapshot
                    .disabled()
                    .unwrap_or_else(|| snapshot.focused().is_none()),
            )
        })
        .expect("test window should expose the button")
    }

    #[test]
    fn position_is_formatted_as_minutes_and_seconds() {
        assert_eq!(clock(Duration::ZERO), "00:00");
        assert_eq!(clock(Duration::from_millis(59_999)), "00:59");
        assert_eq!(clock(Duration::from_secs(84)), "01:24");
        assert_eq!(clock(Duration::from_secs(75 * 60 + 3)), "75:03");
        assert_eq!(
            format_progress(Duration::from_secs(84), Some(Duration::from_secs(225))),
            "01:24 / 03:45"
        );
        assert_eq!(
            format_progress(Duration::from_secs(84), None),
            "01:24 (duration unavailable)"
        );
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn paste_enter_duplicate_play_and_stop(cx: &mut TestAppContext) {
        let (window, view, mut received, states) = open(cx, state(Phase::Ready));
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                "https://youtu.be/aqz-KE-bpKQ".into(),
            ));
            window.press("ctrl-v", cx);
        })
        .expect("test window should accept clipboard paste");
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.press("enter", cx);
            window.press("enter", cx);
            window.click("play", cx);
        })
        .expect("test window should dispatch Enter and Play");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/aqz-KE-bpKQ")
        );
        assert!(
            received.try_recv().is_err(),
            "busy UI must reject duplicate Play"
        );
        click(cx, window, "stop");
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        states.send_replace(state(Phase::Ready));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            window.press("ctrl-a", cx);
            window.input("https://music.youtube.com/watch?v=BaW_jenozKc", cx);
            window.click("play", cx);
        })
        .expect("test window should accept a replacement URL and Play");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url.contains("music.youtube.com"))
        );
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Connecting));
    }

    #[gpui_kit::test]
    fn pause_resume_button_follows_controller_state(cx: &mut TestAppContext) {
        let (window, _view, mut received, states) =
            open(cx, playing(Phase::Playing, Some("Song"), 84));
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        // Rapid repeat clicks are ignored until the controller answers.
        click(cx, window, "pause-resume");
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), true));
        states.send_replace(playing(Phase::Paused, Some("Song"), 85));
        cx.run_until_parked();
        assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Resume)));
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
        states.send_replace(playing(Phase::Playing, Some("Song"), 85));
        cx.run_until_parked();
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
    }

    #[gpui_kit::test]
    fn failed_pause_never_leaves_a_false_paused_state(cx: &mut TestAppContext) {
        let (window, view, mut received, states) =
            open(cx, playing(Phase::Playing, Some("Song"), 12));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        // The GUI did not guess Paused while waiting.
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Playing));
        states.send_replace(State {
            error: Some("Could not pause playback. The track is no longer active.".into()),
            ..state(Phase::Error)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(app.state.phase, Phase::Error);
            assert!(app.state.visible_track().is_none());
            assert!(!app.control_pending);
        });
        assert!(button(cx, window, "pause-resume").1, "no track to pause");
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn stop_works_while_paused(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) =
            open(cx, playing(Phase::Paused, Some("Song"), 30));
        assert!(!button(cx, window, "stop").1);
        click(cx, window, "stop");
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Stopping));
        assert!(button(cx, window, "pause-resume").1);
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn new_url_replaces_a_paused_track(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) =
            open(cx, playing(Phase::Paused, Some("Old"), 30));
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            window.input("https://youtu.be/BaW_jenozKc", cx);
            window.click("play", cx);
        })
        .expect("test window should accept a URL while paused");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/BaW_jenozKc")
        );
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(app.state.phase, Phase::Connecting);
            assert!(app.state.visible_track().is_none() && app.state.track.is_none());
        });
        assert!(button(cx, window, "pause-resume").1);
    }

    #[gpui_kit::test]
    fn missing_title_duration_and_long_titles_render(cx: &mut TestAppContext) {
        let (window, view, _received, states) = open(cx, playing(Phase::Playing, None, 5));
        let long = "Very long title ".repeat(80);
        for track in [
            TrackInfo {
                title: None,
                duration: None,
            },
            TrackInfo {
                title: Some(long.clone()),
                duration: Some(Duration::from_hours(3)),
            },
            TrackInfo {
                title: Some("ż".repeat(5000)),
                duration: None,
            },
        ] {
            states.send_replace(State {
                track: Some(track),
                error: Some(long.clone()),
                ..playing(Phase::Paused, None, 3725)
            });
            cx.run_until_parked();
            cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
            // Layout must survive the frame; the window never grows or panics.
            assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
        }
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn unavailable_configuration_disables_actions_but_keeps_input_usable(cx: &mut TestAppContext) {
        let (commands, mut received) = mpsc::channel(4);
        let (volume, _) = watch::channel(VolumeRequest::default());
        let (_, updates) = watch::channel(State {
            phase: Phase::Error,
            error: Some("Set DISCORD_BOT_TOKEN in .env, then restart.".into()),
            available: false,
            ..State::default()
        });
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| PlayerApp::new(window, cx, commands, volume, updates))
            })
            .expect("test window should open")
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("url", cx);
            window.input("https://youtu.be/aqz-KE-bpKQ", cx);
            window.press("enter", cx);
            window.click("play", cx);
            window.click("pause-resume", cx);
            window.click("stop", cx);
        })
        .expect("test window should accept input and disabled actions");
        assert!(received.try_recv().is_err());
        cx.update(|cx| {
            assert_eq!(
                view.read(cx).input.read(cx).value(),
                "https://youtu.be/aqz-KE-bpKQ"
            );
        });
    }

    // ---- queue, history, volume --------------------------------------------------------

    fn queue_item(id: u64) -> QueueItem {
        QueueItem {
            id,
            url: format!("https://www.youtube.com/watch?v={id:0>11}"),
            title: Some(format!("Queued {id}")),
        }
    }

    fn history_entry(index: usize) -> HistoryEntry {
        HistoryEntry {
            url: format!("https://www.youtube.com/watch?v={index:0>11}"),
            title: Some(format!("Played {index}")),
            duration_secs: Some(225),
        }
    }

    fn with_lists(phase: Phase, queued: u64, played: usize) -> State {
        State {
            queue: (10..10 + queued).map(queue_item).collect(),
            history: (0..played).map(history_entry).collect(),
            ..state(phase)
        }
    }

    fn present(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        cx.update_window(window, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap_or(false)
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn bounds(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        id: impl Into<ElementId>,
    ) -> Bounds<gpui_kit::Pixels> {
        let id = id.into();
        cx.update_window(window, move |_, window, cx| {
            window.render_frame(cx);
            window.find(id).bounds()
        })
        .expect("test window should expose the element")
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn act(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        action: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| action(window, cx))
            .expect("test window should accept the interaction");
    }

    fn show(cx: &TestAppContext, view: &Entity<PlayerApp>, section: Section) {
        cx.update(|cx| {
            view.update(cx, |app, cx| app.select_section(section, cx));
        });
    }

    fn slider_id(
        cx: &TestAppContext,
        view: &Entity<PlayerApp>,
    ) -> (&'static str, gpui_kit::EntityId) {
        cx.update(|cx| ("slider", view.read(cx).volume.entity_id()))
    }

    fn shown_volume(cx: &TestAppContext, view: &Entity<PlayerApp>) -> f32 {
        cx.update(|cx| view.read(cx).volume.read(cx).value().end())
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn add_to_queue_button_and_ctrl_enter_do_not_play(cx: &mut TestAppContext) {
        let (window, view, mut received, states) = open(cx, state(Phase::Ready));
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            window.input("https://youtu.be/aqz-KE-bpKQ", cx);
            window.click("enqueue", cx);
            window.press("ctrl-enter", cx);
        })
        .expect("test window should accept a URL and Add to queue");
        assert!(
            matches!(received.try_recv(), Ok(Command::Enqueue(url)) if url == "https://youtu.be/aqz-KE-bpKQ")
        );
        assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
        assert!(received.try_recv().is_err());
        // Queueing never changes what the window believes about playback.
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Ready));
        // It stays available while another request is in flight, but not before Discord is up.
        for phase in [
            Phase::Connecting,
            Phase::Resolving,
            Phase::Playing,
            Phase::Paused,
        ] {
            states.send_replace(state(phase));
            cx.run_until_parked();
            assert!(!button(cx, window, "enqueue").1, "{phase:?}");
        }
        states.send_replace(State {
            available: false,
            ..state(Phase::Error)
        });
        cx.run_until_parked();
        assert!(button(cx, window, "enqueue").1);
        click(cx, window, "enqueue");
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn next_follows_the_queue_and_ignores_rapid_clicks(cx: &mut TestAppContext) {
        let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 0, 0));
        assert!(button(cx, window, "next").1, "nothing queued");
        click(cx, window, "next");
        assert!(received.try_recv().is_err());
        states.send_replace(with_lists(Phase::Ready, 2, 0));
        cx.run_until_parked();
        assert_eq!(button(cx, window, "next"), ("Next".into(), false));
        click(cx, window, "next");
        click(cx, window, "next");
        assert!(matches!(received.try_recv(), Ok(Command::Next)));
        assert!(received.try_recv().is_err(), "a double click is one Next");
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Connecting));
        assert!(button(cx, window, "next").1);
        // The controller's answer re-enables it for the remaining entry.
        states.send_replace(with_lists(Phase::Resolving, 1, 0));
        cx.run_until_parked();
        assert!(
            !button(cx, window, "next").1,
            "Next works during extraction too"
        );
        for phase in [Phase::Stopping, Phase::Initializing] {
            states.send_replace(with_lists(phase, 1, 0));
            cx.run_until_parked();
            assert!(button(cx, window, "next").1, "{phase:?}");
        }
    }

    #[gpui_kit::test]
    fn tabs_switch_between_queue_and_history(cx: &mut TestAppContext) {
        let (window, view, _received, states) = open(cx, with_lists(Phase::Ready, 2, 3));
        assert!(present(cx, window, ("queue-row", 10_u64)));
        assert!(!present(cx, window, ("history-row", 0_usize)));
        assert!(bounds(cx, window, "queue-count").size.width > px(0.));
        act(cx, window, |window, cx| {
            window.within("sections").click(1_usize, cx);
        });
        cx.update(|cx| assert_eq!(view.read(cx).section, Section::History));
        assert!(present(cx, window, ("history-row", 0_usize)));
        assert!(!present(cx, window, ("queue-row", 10_u64)));
        assert!(present(cx, window, "history-count"));
        act(cx, window, |window, cx| {
            window.within("sections").click(0_usize, cx);
        });
        cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
        assert!(present(cx, window, ("queue-row", 11_u64)));
        // The tab title carries the pending count and follows controller updates.
        states.send_replace(with_lists(Phase::Ready, 0, 3));
        cx.run_until_parked();
        assert!(present(cx, window, "queue-empty"));
    }

    #[gpui_kit::test]
    fn ctrl_digits_switch_sections_from_the_keyboard(cx: &mut TestAppContext) {
        let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 1, 1));
        act(cx, window, |window, cx| window.press("ctrl-2", cx));
        cx.update(|cx| assert_eq!(view.read(cx).section, Section::History));
        act(cx, window, |window, cx| window.press("ctrl-1", cx));
        cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
        // Plain digits belong to the URL field.
        act(cx, window, |window, cx| window.press("2", cx));
        cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
    }

    #[gpui_kit::test]
    fn queue_rows_send_their_own_commands(cx: &mut TestAppContext) {
        let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Playing, 3, 0));
        assert!(
            button(cx, window, ("queue-up", 10_u64)).1,
            "first row cannot move up"
        );
        assert!(
            button(cx, window, ("queue-down", 12_u64)).1,
            "last row cannot move down"
        );
        assert!(!button(cx, window, ("queue-up", 11_u64)).1);
        click(cx, window, ("queue-up", 10_u64));
        assert!(received.try_recv().is_err());
        click(cx, window, ("queue-up", 11_u64));
        assert!(matches!(
            received.try_recv(),
            Ok(Command::Shift { id: 11, up: true })
        ));
        click(cx, window, ("queue-down", 10_u64));
        assert!(matches!(
            received.try_recv(),
            Ok(Command::Shift { id: 10, up: false })
        ));
        click(cx, window, ("queue-remove", 12_u64));
        assert!(matches!(received.try_recv(), Ok(Command::Dequeue(12))));
        click(cx, window, "queue-clear");
        assert!(matches!(received.try_recv(), Ok(Command::ClearQueue)));
        // Row actions never touch playback, so the window keeps showing it.
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn an_empty_queue_explains_itself_and_disables_clear(cx: &mut TestAppContext) {
        let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 0));
        assert!(present(cx, window, "queue-empty"));
        assert!(button(cx, window, "queue-clear").1);
        click(cx, window, "queue-clear");
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn history_rows_play_queue_and_forget(cx: &mut TestAppContext) {
        let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 0, 3));
        show(cx, &view, Section::History);
        let url = history_entry(1).url;
        click(cx, window, ("history-queue", 1_usize));
        assert!(matches!(received.try_recv(), Ok(Command::Enqueue(sent)) if sent == url));
        click(cx, window, ("history-remove", 1_usize));
        assert!(matches!(received.try_recv(), Ok(Command::ForgetHistory(sent)) if sent == url));
        click(cx, window, ("history-play", 1_usize));
        assert!(matches!(received.try_recv(), Ok(Command::Play(sent)) if sent == url));
        // Play now replaces the track like Play URL: the window waits for the controller.
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Connecting));
        // While a request is in flight Play now is unavailable; queueing still works.
        assert!(button(cx, window, ("history-play", 0_usize)).1);
        assert!(!button(cx, window, ("history-queue", 0_usize)).1);
        click(cx, window, ("history-play", 0_usize));
        assert!(received.try_recv().is_err());
        states.send_replace(with_lists(Phase::Playing, 0, 3));
        cx.run_until_parked();
        assert!(
            !button(cx, window, ("history-play", 0_usize)).1,
            "replaces a playing track"
        );
    }

    #[gpui_kit::test]
    fn clearing_history_asks_first(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 3));
        show(cx, &view, Section::History);
        assert_eq!(button(cx, window, "history-clear").0, "Clear history");
        click(cx, window, "history-clear");
        assert!(received.try_recv().is_err(), "the first click only asks");
        assert_eq!(button(cx, window, "history-clear").0, "Yes, clear");
        click(cx, window, "history-clear-cancel");
        assert_eq!(button(cx, window, "history-clear").0, "Clear history");
        assert!(received.try_recv().is_err());
        // Leaving the tab also forgets the question.
        click(cx, window, "history-clear");
        show(cx, &view, Section::Queue);
        show(cx, &view, Section::History);
        assert_eq!(button(cx, window, "history-clear").0, "Clear history");
        click(cx, window, "history-clear");
        click(cx, window, "history-clear");
        assert!(matches!(received.try_recv(), Ok(Command::ClearHistory)));
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn an_empty_history_explains_itself(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 0));
        show(cx, &view, Section::History);
        assert!(present(cx, window, "history-empty"));
        assert!(button(cx, window, "history-clear").1);
        click(cx, window, "history-clear");
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn a_recovered_history_file_is_reported(cx: &mut TestAppContext) {
        let (window, view, _received, _states) = open(
            cx,
            State {
                notice: Some("Saved history was unreadable; starting empty.".into()),
                ..with_lists(Phase::Ready, 0, 0)
            },
        );
        show(cx, &view, Section::History);
        assert!(present(cx, window, "notice"));
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn long_lists_scroll_inside_the_window(cx: &mut TestAppContext) {
        let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 100, 50));
        let last = ("queue-row", 109_u64);
        let header = bounds(cx, window, "queue-count");
        let before = bounds(cx, window, last);
        assert!(
            before.origin.y > header.origin.y + px(400.),
            "row lies below the viewport"
        );
        cx.update_window(window, |_, window, cx| {
            window.scroll(
                ("queue-row", 10_u64),
                ScrollDelta::Pixels(point(px(0.), px(-4000.))),
                cx,
            );
        })
        .expect("test window should scroll");
        let after = bounds(cx, window, last);
        assert!(
            after.origin.y < before.origin.y - px(300.),
            "the list moved"
        );
        assert!(after.origin.y < px(560.), "the last row is now on screen");
        show(cx, &view, Section::History);
        let before = bounds(cx, window, ("history-row", 49_usize));
        cx.update_window(window, |_, window, cx| {
            window.scroll(
                ("history-row", 0_usize),
                ScrollDelta::Pixels(point(px(0.), px(-4000.))),
                cx,
            );
        })
        .expect("test window should scroll");
        assert!(bounds(cx, window, ("history-row", 49_usize)).origin.y < before.origin.y);
        // The controls above the lists never move.
        assert!(button(cx, window, "pause-resume").1);
    }

    #[gpui_kit::test]
    fn long_titles_and_errors_keep_the_layout(cx: &mut TestAppContext) {
        let long = "Very long title ".repeat(60);
        let mut initial = with_lists(Phase::Error, 2, 2);
        initial.queue[0].title = Some(long.clone());
        initial.history[0].title = Some("ż".repeat(4000));
        initial.error = Some(long);
        let (window, view, _received, _states) = open(cx, initial);
        let window_width = px(520.);
        let title = bounds(cx, window, ("queue-title", 10_u64));
        assert!(title.size.width < window_width, "queue title is truncated");
        assert_eq!(
            bounds(cx, window, ("queue-row", 10_u64)).size.height,
            bounds(cx, window, ("queue-row", 11_u64)).size.height
        );
        let error = bounds(cx, window, "error");
        assert!(
            error.size.height < px(70.),
            "error is clamped to a few lines"
        );
        assert!(error.size.width <= window_width);
        for id in ["pause-resume", "next", "stop", "play", "enqueue"] {
            let button = bounds(cx, window, id);
            assert!(
                button.bottom() < px(560.) && button.right() <= window_width,
                "{id} is clipped"
            );
        }
        show(cx, &view, Section::History);
        let title = bounds(cx, window, ("history-title", 0_usize));
        assert!(
            title.size.width < window_width,
            "history title is truncated"
        );
        // Both rows keep their height whatever the title length.
        assert_eq!(
            bounds(cx, window, ("history-row", 0_usize)).size.height,
            bounds(cx, window, ("history-row", 1_usize)).size.height
        );
    }

    #[gpui_kit::test]
    fn focus_leaves_a_control_that_turns_off(cx: &mut TestAppContext) {
        let (window, view, _received, states) = open(cx, playing(Phase::Playing, Some("Song"), 3));
        let on_stop = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, _| window.find("stop").focused())
                .ok()
                .flatten()
                .unwrap_or(false)
        };
        for _ in 0..12 {
            if on_stop(cx) {
                break;
            }
            act(cx, window, |window, cx| window.press("tab", cx));
        }
        assert!(on_stop(cx), "Tab reaches Stop");
        // Stop turns off; typing focus must not stay on the dead button.
        states.send_replace(state(Phase::Ready));
        cx.run_until_parked();
        assert!(!on_stop(cx));
        let input_focused = cx.update_window(window, |_, window, cx| {
            view.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert_eq!(input_focused.ok(), Some(true));
    }

    #[gpui_kit::test]
    fn stopping_from_the_keyboard_returns_focus_to_the_input(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) =
            open(cx, playing(Phase::Playing, Some("Song"), 3));
        let on_stop = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, _| window.find("stop").focused())
                .ok()
                .flatten()
                .unwrap_or(false)
        };
        for _ in 0..12 {
            if on_stop(cx) {
                break;
            }
            act(cx, window, |window, cx| window.press("tab", cx));
        }
        assert!(on_stop(cx));
        act(cx, window, |window, cx| window.press("enter", cx));
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        let input_focused = cx.update_window(window, |_, window, cx| {
            view.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert_eq!(input_focused.ok(), Some(true));
    }

    #[gpui_kit::test]
    fn the_url_input_has_focus_and_keeps_it(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) = open(cx, state(Phase::Ready));
        let focused = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, cx| {
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            })
            .unwrap_or(false)
        };
        assert!(focused(cx), "the input starts focused");
        click(cx, window, "enqueue");
        assert!(received.try_recv().is_ok());
        click(cx, window, "next");
        assert!(focused(cx), "buttons do not steal typing focus");
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn the_slider_follows_mouse_drags_and_commits_on_release(cx: &mut TestAppContext) {
        let ((window, view, _received, _states), volume) = open_full(cx, state(Phase::Playing));
        let slider = slider_id(cx, &view);
        assert!(
            (shown_volume(cx, &view) - 100.0).abs() < f32::EPSILON,
            "existing users keep full volume"
        );
        let track = bounds(cx, window, slider);
        // A click anywhere on the track jumps there and is final.
        cx.update_window(window, |_, window, cx| window.click(slider, cx))
            .expect("test window should click the slider");
        let request = *volume.borrow();
        assert!((40..=60).contains(&request.percent), "{request:?}");
        assert!(request.commit && request.seq >= 1);
        // A drag emits live values, but only the newest is kept, and it commits on release.
        let y = track.center().y;
        cx.update_window(window, |_, window, cx| {
            window.drag(
                point(track.center().x, y),
                point(track.left() + track.size.width * 0.2, y),
                cx,
            );
        })
        .expect("test window should drag the slider");
        let request = *volume.borrow();
        assert!((15..=25).contains(&request.percent), "{request:?}");
        assert!(request.commit);
        assert!(
            request.seq >= 3,
            "intermediate values were produced, then coalesced"
        );
        assert!((shown_volume(cx, &view) - f32::from(request.percent)).abs() < f32::EPSILON);
    }

    #[gpui_kit::test]
    fn the_slider_works_from_the_keyboard(cx: &mut TestAppContext) {
        let ((window, view, _received, _states), volume) = open_full(cx, state(Phase::Paused));
        let slider = slider_id(cx, &view);
        // Clicking the slider focuses it; start from a known value.
        act(cx, window, |window, cx| {
            window.click(slider, cx);
            view.update(cx, |app, cx| {
                app.volume
                    .update(cx, |slider, cx| slider.set_value(50.0, window, cx));
            });
        });
        let press = |cx: &mut TestAppContext, key: &'static str| {
            act(cx, window, move |window, cx| window.press(key, cx));
            *volume.borrow()
        };
        assert_eq!(press(cx, "right").percent, 55);
        assert_eq!(press(cx, "up").percent, 60);
        assert_eq!(press(cx, "left").percent, 55);
        assert_eq!(press(cx, "down").percent, 50);
        assert_eq!(press(cx, "pageup").percent, 60);
        assert_eq!(press(cx, "pagedown").percent, 50);
        assert_eq!(press(cx, "home").percent, 0);
        assert_eq!(press(cx, "left").percent, 0, "never below zero");
        let request = press(cx, "end");
        assert_eq!(request.percent, 100);
        assert_eq!(press(cx, "right").percent, 100, "never above one hundred");
        assert!(request.commit, "key presses are final values");
        let seq = press(cx, "x").seq;
        assert_eq!(press(cx, "a").seq, seq, "other keys are ignored");
        assert!((shown_volume(cx, &view) - 100.0).abs() < f32::EPSILON);
        assert_eq!(volume.borrow().percent, 100);
    }

    #[gpui_kit::test]
    fn the_slider_shows_what_the_controller_applied(cx: &mut TestAppContext) {
        let ((window, view, _received, states), volume) = open_full(cx, state(Phase::Playing));
        let slider = slider_id(cx, &view);
        // Unprompted changes (restored preference) move the slider.
        states.send_replace(State {
            volume: 20,
            ..state(Phase::Playing)
        });
        cx.run_until_parked();
        assert!((shown_volume(cx, &view) - 20.0).abs() < f32::EPSILON);
        act(cx, window, |window, cx| {
            window.click(slider, cx);
            window.press("left", cx);
        });
        let seq = volume.borrow().seq;
        let wanted = shown_volume(cx, &view);
        // A snapshot from before the controller saw the request must not undo it.
        states.send_replace(State {
            volume: 20,
            volume_seq: seq - 1,
            ..state(Phase::Playing)
        });
        cx.run_until_parked();
        assert!((shown_volume(cx, &view) - wanted).abs() < f32::EPSILON);
        // When it answers having refused (track gone), the slider goes back.
        states.send_replace(State {
            volume: 20,
            volume_seq: seq,
            ..state(Phase::Error)
        });
        cx.run_until_parked();
        assert!((shown_volume(cx, &view) - 20.0).abs() < f32::EPSILON);
        assert_eq!(view_text(cx, &view), "20%");
    }

    fn view_text(cx: &TestAppContext, view: &Entity<PlayerApp>) -> String {
        cx.update(|cx| format!("{}%", view.read(cx).volume.read(cx).value().end().round()))
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn the_slider_is_inert_without_discord(cx: &mut TestAppContext) {
        let ((window, view, _received, _states), volume) = open_full(
            cx,
            State {
                available: false,
                ..state(Phase::Error)
            },
        );
        let slider = slider_id(cx, &view);
        cx.update_window(window, |_, window, cx| {
            window.click(slider, cx);
            window.press("left", cx);
            window.press("home", cx);
        })
        .expect("test window should accept input");
        assert_eq!(volume.borrow().seq, 0);
        assert_eq!(volume.borrow().percent, 100);
    }
}
