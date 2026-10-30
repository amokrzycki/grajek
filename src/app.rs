pub(super) mod composer;
mod controls;
mod help;
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
    slider::{SliderEvent, SliderState},
    tab::{Tab, TabBar},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Task, TestSupportExt, Window, div, px,
    relative,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::player::{Command, Phase, State, VolumeRequest};
use crate::style::space;
use help::HelpExt;

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
    help: help::Help,
    token_error: bool,
    token: Entity<InputState>,
    settings_open: bool,
    credentials_open: bool,
    preferences_open: bool,
    server_select: settings::DestinationSelect,
    channel_select: settings::DestinationSelect,
    settings_focus: FocusHandle,
    settings_scroll: gpui_kit::ScrollHandle,
    confirm_setup_focus: FocusHandle,
    queue_options_open: bool,
    setup_starting: bool,
    setup_help: bool,
    input: Entity<InputState>,
    /// An Add to queue the controller has not answered yet.
    enqueue_pending: Option<composer::PendingEnqueue>,
    /// The entry the controller last inserted for this window's Add to queue.
    added: Option<u64>,
    local_hint: Option<&'static str>,
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
    subscriptions: Vec<Subscription>,
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
    #[expect(
        clippy::too_many_lines,
        reason = "The window owns its controls, subscriptions and focus handles in one constructor"
    )]
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: mpsc::Sender<Command>,
        volume_requests: watch::Sender<VolumeRequest>,
        mut states: watch::Receiver<State>,
    ) -> Self {
        lists::init(cx);
        controls::init(cx);
        let help = help::Help::new(cx);
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
            cx.observe_window_activation(window, |this, _, cx| this.help.dismiss(cx)),
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { secondary, .. } => {
                    // Enter plays; Ctrl+Enter queues.
                    if *secondary {
                        this.enqueue(cx);
                    } else {
                        this.play(window, cx);
                    }
                }
                InputEvent::Change => {
                    // Typing starts a new entry; programmatic clearing keeps the confirmation.
                    if !this.input_text(cx).is_empty() {
                        this.added = None;
                        this.local_hint = None;
                        this.enqueue_pending = None;
                    }
                    cx.notify();
                }
                _ => {}
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
        let (server_select, channel_select) = settings::selectors(window, cx);
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
            help,
            token_error: false,
            token,
            settings_open: true,
            credentials_open: false,
            preferences_open: false,
            server_select,
            channel_select,
            settings_focus: cx.focus_handle(),
            settings_scroll: gpui_kit::ScrollHandle::default(),
            confirm_setup_focus: cx.focus_handle(),
            queue_options_open: false,
            setup_starting: true,
            setup_help: false,
            input,
            enqueue_pending: None,
            added: None,
            local_hint: None,
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
            subscriptions,
            _updates: updates,
            #[cfg(target_os = "windows")]
            media: None,
        };
        app.subscribe_selectors(window, cx);
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
        let setup_changed = self.state.setup != state.setup;
        let confirmation_changed = self.state.confirm_setup != state.confirm_setup;
        let authenticated = self.state.setup.bot.is_none() && state.setup.bot.is_some();
        let credentials_removed = self.state.setup.bot.is_some() && state.setup.bot.is_none();
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
        }
        let resolution = self.resolve_enqueue(&state);
        let focused_row = self
            .queue_scroll
            .rows
            .iter()
            .position(|(_, focus)| focus.contains_focused(window, cx))
            .and_then(|index| {
                let id = self.state.queue.get(index)?.id;
                let action = self
                    .queue_scroll
                    .actions
                    .get(&ElementId::from(("queue-row", id)))
                    .and_then(|trackers| {
                        trackers
                            .iter()
                            .position(|tracker| tracker.contains_focused(window, cx))
                    });
                Some((index, id, action))
            });
        self.state = state;
        if let Some(resolution) = &resolution {
            self.finish_enqueue(resolution, window, cx);
        }
        if authenticated {
            self.credentials_open = false;
        }
        if setup_changed {
            self.sync_selectors(window, cx);
        }
        if self.settings_open && (confirmation_changed || authenticated || credentials_removed) {
            self.focus_settings(window, cx);
        }
        self.import_pending = false;
        if self.state.transport_ack >= self.transport_seq {
            self.pending = None;
        }
        self.sync_lists(cx);
        let lost_row =
            focused_row.filter(|(_, id, _)| self.state.queue.iter().all(|item| item.id != *id));
        // A move can leave the focused arrow disabled (first slot: Move up; last: Move down).
        // A disabled control cannot hold focus, so hand it to the opposite arrow.
        let stranded = focused_row.and_then(|(_, id, action)| {
            let index = self.state.queue.iter().position(|item| item.id == id)?;
            let action = action.filter(|action| *action < 2)?;
            (!self.row_actions(Section::Queue, index)[action]).then_some((index, 1 - action))
        });
        // Gateway loss also disables preferences outside the transport group.
        if let Some((index, action)) = stranded
            && self.section == Section::Queue
        {
            self.focus_row_action(Section::Queue, index, action, window, cx);
        } else if let Some((index, _, _)) = lost_row
            && self.section == Section::Queue
            && !self.state.queue.is_empty()
        {
            // The focused entry left the queue: keep keyboard focus in the list, on the same slot.
            self.focus_row_action(
                Section::Queue,
                index.min(self.state.queue.len() - 1),
                2,
                window,
                cx,
            );
        } else if lost_gateway
            || lost_voice_control
            || import_finished
            || lost_progress
            || lost_row.is_some()
        {
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
    if state.setup.bot.is_none()
        && matches!(state.setup.status, crate::setup::Status::NeedsToken)
        && !state.available
    {
        "Bot setup needed"
    } else if matches!(state.setup.status, crate::setup::Status::NeedsDestination) {
        "Choose a voice channel"
    } else if state.phase == Phase::Disconnecting {
        "Leaving Discord voice…"
    } else if state.available && state.voice_connected {
        "Voice connected"
    } else if state.available {
        "Voice idle"
    } else if state.phase == Phase::Initializing {
        "Connecting to Discord…"
    } else {
        "Discord unavailable"
    }
}

fn connection_color(state: &State, theme: &gpui_kit::component::Theme) -> gpui_kit::Hsla {
    match (&state.setup.status, state.phase, state.available) {
        (_, _, true) => theme.green,
        (
            crate::setup::Status::NeedsToken
            | crate::setup::Status::NeedsDestination
            | crate::setup::Status::Checking,
            _,
            _,
        )
        | (_, Phase::Initializing, _) => theme.muted_foreground,
        _ => theme.danger,
    }
}

/// A one-line, ellipsized label whose full text appears in a tooltip.
fn label(id: impl Into<ElementId>, text: String, help: &help::Help) -> impl IntoElement {
    let tip = text.clone();
    label_with_tooltip(id, text, tip, help)
}

fn label_with_tooltip(
    id: impl Into<ElementId>,
    text: String,
    tip: String,
    help: &help::Help,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .truncate()
        .aria_label(text.clone())
        .child(text)
        .help(tip, help)
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
        let color = connection_color(&self.state, theme);
        div()
            .id("player-header")
            .test_support()
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
                                    .child(label("voice-status", status.into(), &self.help)),
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
                                &self.help,
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
                    .when(self.state.can_leave_voice(), |row| {
                        row.child(self.leave_button(cx))
                    })
                    .child(
                        Button::new("settings")
                            .icon(settings_icon)
                            .accessibility_label(settings_label)
                            .help(settings_label, &self.help)
                            .ghost()
                            .flex_none()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.help.dismiss(cx);
                                this.setup_starting = false;
                                this.settings_open = !this.settings_open;
                                this.queue_options_open = false;
                                if this.settings_open {
                                    this.focus_settings(window, cx);
                                } else {
                                    window.focus(&this.input.read(cx).focus_handle(cx), cx);
                                }
                                cx.notify();
                            })),
                    ),
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
                        .child(label("title", title, &self.help)),
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
        let tip = error.clone();
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
                .help(tip, &self.help)
                .test_support(),
        )
    }

    fn idle(&self) -> bool {
        self.state.visible_track().is_none()
            && matches!(self.state.phase, Phase::Ready | Phase::Error)
            && !matches!(self.pending, Some(Pending::Play | Pending::Next))
    }

    fn active_player(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("active-player")
            .flex_none()
            .flex()
            .flex_col()
            .gap(space::TIGHT)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::TIGHT)
                    .child(self.now_playing(cx))
                    .children(self.progress_row(window, cx)),
            )
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
            .children(self.error(cx))
            .test_support()
    }

    /// Idle composition. Setup tasks keep a headline and one action. Once the bot can play, the
    /// listening slot holds only what is useful: Play Queue beside volume when entries are
    /// pending, or the volume alone, right-aligned in the same slot, when the queue is empty
    /// (volume can be set before anything plays). No headline, no disabled transport.
    fn idle_player(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let error = self.error(cx);
        let panel = div().id("idle-player").flex().flex_none().flex_col();
        if self.state.available {
            panel
                .gap(space::GROUP)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(space::DETAIL)
                        .child(if self.state.queue.is_empty() {
                            div().into_any_element()
                        } else {
                            self.play_queue_button(cx).into_any_element()
                        })
                        .child(
                            div()
                                .w(px(120.))
                                .flex_none()
                                .child(self.volume_row(window, cx)),
                        ),
                )
                .children(error)
                .test_support()
                .into_any_element()
        } else {
            let (title, guidance, action) = if self.state.setup.bot.is_none() {
                (
                    "Connect your Discord bot",
                    "Set up a bot to play YouTube in your Discord voice channel.",
                    "Connect a bot",
                )
            } else {
                (
                    "Choose where to play",
                    "Select a server and voice channel in Discord settings.",
                    "Choose destination",
                )
            };
            panel
                .gap(space::DETAIL)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::TIGHT)
                        .child(
                            div()
                                .text_size(px(24.))
                                .line_height(px(30.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label("title", title.into(), &self.help)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .line_height(px(20.))
                                .text_color(cx.theme().muted_foreground)
                                .child(guidance),
                        ),
                )
                .child(
                    div().flex().child(
                        Button::new("setup-next-step")
                            .primary()
                            .label(action)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.setup_starting = false;
                                this.settings_open = true;
                                this.help.dismiss(cx);
                                this.focus_settings(window, cx);
                                cx.notify();
                            })),
                    ),
                )
                .children(error)
                .test_support()
                .into_any_element()
        }
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
            .id("queue-region")
            .test_support()
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
        self.help
            .enable(!self.settings_open && !self.queue_options_open, cx);
        let help = self.help.clone();
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
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        if this.state.confirm_setup {
                            this.send(Command::ConfirmSetup(false));
                        } else if this.credentials_open {
                            this.credentials_open = false;
                            this.focus_settings(window, cx);
                        } else {
                            this.settings_open = false;
                            this.input.read(cx).focus_handle(cx).focus(window, cx);
                        }
                        this.help.dismiss(cx);
                        cx.notify();
                    }
                }))
                .child(self.header(cx))
                .child(self.settings(cx))
                .child(self.help.overlay.clone())
                .into_any_element();
        }
        // Related controls sit 4–8px apart; the three jobs (listening, adding music, up next)
        // are separated by larger, unequal gaps so they read as regions without borders.
        let compact = window.viewport_size().height <= px(480.);
        let (after_header, before_composer, before_queue) = if compact {
            (px(8.), px(12.), px(8.))
        } else {
            (px(12.), px(20.), px(16.))
        };
        let listening = if self.idle() {
            self.idle_player(window, cx)
        } else {
            self.active_player(window, cx).into_any_element()
        };
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p(space::INSET)
            .text_sm()
            .flex()
            .flex_col()
            .overflow_hidden()
            .capture_any_mouse_down(move |_, _, cx| help.dismiss(cx))
            .capture_key_down({
                let help = self.help.clone();
                move |event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key != "f1" {
                        help.dismiss(cx);
                    }
                }
            })
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
            .child(div().mt(after_header).child(listening))
            .child(div().mt(before_composer).child(self.composer(window, cx)))
            .child(
                div()
                    .mt(before_queue)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.sections(cx)),
            )
            .child(self.help.overlay.clone())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
