mod components;
pub(super) mod composer;
mod favorites;
pub(super) mod fmt;
mod history;
mod motion;
mod progress;
mod queue;
mod search;
mod sections;
mod settings;
mod transport;
mod ui;
mod volume;

use gpui_kit::component::{
    ActiveTheme,
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Task,
    TestSupportExt, Window, div, px,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::player::{Command, State, VolumeRequest};
use crate::style::space;

use components::Panel;
use composer::{Landing, PendingEnqueue, Resolution};
use motion::{Frame, Motion};
use sections::TrackTarget;
use sections::lists::ListNavigation;
use sections::{CollectionSelect, Section};
use settings::DestinationSelect;
use transport::Pending;
use ui::help::HelpExt;
use ui::{FocusPresentation, Help};

#[expect(
    clippy::struct_excessive_bools,
    reason = "Independent UI focus, pending-operation and panel flags are not a single state machine"
)]
pub struct PlayerApp {
    components: Panel,
    help: Help,
    focus: FocusPresentation,
    row_menu: Option<TrackTarget>,
    token_error: bool,
    token: Entity<InputState>,
    settings_open: bool,
    credentials_open: bool,
    preferences_open: bool,
    server_select: DestinationSelect,
    channel_select: DestinationSelect,
    settings_focus: FocusHandle,
    settings_scroll: gpui_kit::ScrollHandle,
    confirm_setup_focus: FocusHandle,
    queue_options_open: bool,
    setup_starting: bool,
    setup_help: bool,
    input: Entity<InputState>,
    search_ui: search::SearchUi,
    enqueue_pending: Option<PendingEnqueue>,
    enqueue_seq: u64,
    /// The entry the controller last placed for this window, and how it got there.
    added: Option<(u64, Landing)>,
    local_hint: Option<String>,
    retired_import: Option<crate::playlist::ImportStatus>,
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
    motion: Motion,
    frame: Frame,
    height_reveals: motion::HeightReveals,
    reveal: motion::RevealFrame,
    /// Only a change of these messages starts a fade.
    shown_error: Option<String>,
    shown_notice: Option<String>,
    shown_view: Option<bool>,
    /// Prevent duplicate input while keeping the authoritative playback snapshot visible.
    pending: Option<Pending>,
    transport_seq: u64,
    queue_scroll: ListNavigation,
    history_scroll: ListNavigation,
    favorites_scroll: ListNavigation,
    import_pending: bool,
    favorite_pending: Option<u64>,
    favorite_seq: u64,
    collection_select: CollectionSelect,
    now_playing_favorite_focus: FocusHandle,
    /// The slider is being dragged; controller snapshots must not fight the pointer.
    dragging: bool,
    volume_seq: u64,
    /// The newest volume request the controller has not yet answered.
    volume_pending: Option<u64>,
    section: Section,
    /// "Clear history" was pressed once and awaits confirmation.
    confirm_clear: bool,
    history_header_focus: FocusHandle,
    subscriptions: Vec<Subscription>,
    _updates: Task<()>,
    #[cfg(target_os = "windows")]
    media: Option<crate::platform::media::windows::MediaSession>,
}

impl PlayerApp {
    #[cfg(target_os = "windows")]
    pub fn with_media(
        mut self,
        media: Option<crate::platform::media::windows::MediaSession>,
    ) -> Self {
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
        sections::lists::init(cx);
        ui::menu::init(cx);
        let help = Help::new(cx);
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Bot token")
        });
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("YouTube link or search query"));
        window.focus(&token.read(cx).focus_handle(cx), cx);
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(f32::from(crate::preferences::VOLUME_MAX))
                .step(1.0)
                .default_value(f32::from(volume_requests.borrow().percent))
        });
        let progress = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(progress::SLIDER_MAX)
                .step(1.)
                .default_value(0.)
        });
        let subscriptions = vec![
            cx.observe_window_activation(window, |this, _, cx| this.help.dismiss(cx)),
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { secondary, .. } => {
                    if *secondary {
                        this.enqueue(crate::player::Placement::End, cx);
                    } else {
                        this.play(window, cx);
                    }
                }
                InputEvent::Change => {
                    // Typing starts a new entry; programmatic clearing keeps the confirmation.
                    this.local_hint = None;
                    this.retire_search(cx);
                    if this.state.import != crate::playlist::ImportStatus::Importing {
                        this.retired_import = Some(this.state.import.clone());
                    }
                    if !this.input_text(cx).is_empty() {
                        this.added = None;
                        if this
                            .enqueue_pending
                            .as_ref()
                            .is_some_and(|pending| pending.text.is_some())
                        {
                            this.enqueue_pending = None;
                        }
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
        let collection_select = sections::selector(window, cx);
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
        let favorite_seq = state.favorite_result.as_ref().map_or(0, |r| r.request_id);
        let shown_error = motion::displayed_error(&state);
        let shown_notice = state.notice.clone();
        let mut app = Self {
            components: Panel::new(cx),
            help,
            focus: FocusPresentation::from_window(window),
            row_menu: None,
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
            search_ui: search::SearchUi::default(),
            enqueue_pending: None,
            enqueue_seq: 0,
            added: None,
            local_hint: None,
            retired_import: None,
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
            motion: Motion::default(),
            frame: Frame::default(),
            height_reveals: motion::HeightReveals::default(),
            reveal: motion::RevealFrame::default(),
            shown_error,
            shown_notice,
            shown_view: None,
            pending: None,
            transport_seq,
            queue_scroll: ListNavigation::new(),
            history_scroll: ListNavigation::new(),
            favorites_scroll: ListNavigation::new(),
            import_pending: false,
            favorite_pending: None,
            favorite_seq,
            collection_select,
            now_playing_favorite_focus: cx.focus_handle(),
            dragging: false,
            volume_seq: 0,
            volume_pending: None,
            section: Section::Queue,
            confirm_clear: false,
            history_header_focus: cx.focus_handle(),
            subscriptions,
            _updates: updates,
            #[cfg(target_os = "windows")]
            media: None,
        };
        app.subscribe_selectors(window, cx);
        app.subscribe_collection_selector(window, cx);
        app.sync_collection_selector(window, cx);
        app.sync_lists(cx);
        app.sync_progress(window, cx);
        app
    }

    fn accept_state(&mut self, state: State, window: &mut Window, cx: &mut Context<Self>) {
        self.observe_search(&state, window, cx);
        self.observe_favorite(&state, window, cx);
        if state.components_request != self.state.components_request {
            self.open_components(cx);
        }
        if self.setup_starting && state.controller_ready && !state.setup_busy {
            if state.setup.destination.is_some() {
                self.leave_settings(window, cx);
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
        let import_finished = self.state.import == crate::playlist::ImportStatus::Importing
            && state.import != crate::playlist::ImportStatus::Importing
            && self.import_focus.contains_focused(window, cx);
        let lost_progress = self.state.can_seek()
            && (!state.can_seek() || self.state.track_id != state.track_id)
            && self.progress_focus.is_focused(window);
        if self.state.track_id != state.track_id || !state.can_seek() {
            self.progress_preview = None;
        }
        if lost_gateway {
            self.close_popups();
        }
        let list_focus = self.list_focus(&state, window, cx);
        let resolution = self.resolve_enqueue(&state);
        let moved_focus = matches!(
            resolution,
            Some(Resolution::Placed(id, Landing::Moved)) if self.queue_row_focused(id, window, cx)
        );
        if self.state.import != state.import {
            self.retired_import = None;
        }
        let now = cx.background_executor().now();
        self.motion.observe(&self.state, &state, now);
        self.observe_callouts(&state, now);
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
        self.sync_collection_selector(window, cx);
        if lost_gateway || lost_voice_control || import_finished || lost_progress {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        } else if moved_focus {
            self.focus_row_action(Section::Queue, 0, 1, window, cx);
        } else if !self.restore_list_focus(list_focus, window, cx) {
            self.rescue_focus(before, window, cx);
        }
        self.sync_volume(window, cx);
        self.sync_progress(window, cx);
        cx.notify();
    }

    /// Closes every transient menu so only one popup can own the window at a time.
    fn close_popups(&mut self) {
        self.row_menu = None;
        self.queue_options_open = false;
    }

    /// Fire-and-forget: the controller answers every command with a new snapshot, and the
    /// window only ever renders snapshots.
    fn send(&self, command: Command) -> bool {
        self.commands.try_send(command).is_ok()
    }

    fn input_text(&self, cx: &Context<Self>) -> String {
        self.input.read(cx).value().to_string()
    }
}

impl Render for PlayerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus = FocusPresentation::from_window(window);
        self.frame = self.motion_frame(window, cx);
        if self.components.check_pending {
            self.check_components(cx);
        }
        if self.components.open {
            return self.components_surface(window, cx);
        }
        self.help.enable(
            !self.settings_open && !self.queue_options_open && self.row_menu.is_none(),
            cx,
        );
        let help = self.help.clone();
        let theme = cx.theme();
        if self.settings_open {
            return self.settings_surface(window, cx);
        }
        // Related controls sit 4–8px apart; the three jobs (listening, adding music, up next)
        // are separated by larger, unequal gaps so they read as regions without borders.
        let compact = window.viewport_size().height <= fmt::COMPACT_HEIGHT;
        let (after_header, before_composer, before_queue) = if compact {
            (px(8.), px(12.), px(8.))
        } else {
            (px(12.), px(16.), px(12.))
        };
        let has_listening = !self.idle()
            || !self.state.available
            || !self.state.queue.is_empty()
            || self.error(cx).is_some();
        let listening = if self.idle() {
            self.idle_player(window, cx)
        } else {
            self.active_player(window, cx).into_any_element()
        };
        self.entering_surface(div())
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
                    if !ui::help_key(event.keystroke.key.as_str()) {
                        help.dismiss(cx);
                    }
                }
            })
            // The kit's tabs cannot take focus, so Ctrl+1 / Ctrl+2 switch sections.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.search_escape(event, window, cx);
                if event.keystroke.modifiers.control {
                    match event.keystroke.key.as_str() {
                        "1" => this.activate_section(Section::Queue, window, cx),
                        "2" => this.activate_section(Section::History, window, cx),
                        "3" => this.activate_section(Section::Favorites, window, cx),
                        _ => {}
                    }
                }
            }))
            .child(self.header(cx))
            .children(self.state.notice.as_ref().map(|notice| {
                div()
                    .id("notice")
                    .role(gpui_kit::Role::Alert)
                    .flex_none()
                    .text_xs()
                    .text_color(theme.yellow)
                    .line_clamp(2)
                    .when_some(self.frame.notice_fade(), Styled::opacity)
                    .child(notice.clone())
                    .help(notice.clone(), &self.help)
                    .test_support()
            }))
            .when(has_listening, |root| {
                root.child(div().mt(after_header).child(listening))
            })
            .child(
                div()
                    .mt(if has_listening {
                        before_composer
                    } else {
                        after_header
                    })
                    .child(self.composer(window, cx)),
            )
            .child(
                div()
                    .mt(before_queue)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.sections(window, cx)),
            )
            .child(self.audio_footer(window, cx))
            .child(self.help.overlay.clone())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
