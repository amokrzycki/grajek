use gpui_kit::assets::IconName as SpeakerIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    popover::Popover,
    slider::Slider,
};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, prelude::FluentBuilder, px,
};

use super::help::HelpExt;

use super::{Pending, PlayerApp};
use crate::player::{Command, ImportStatus, Phase, RepeatMode, VolumeRequest, percent_from};
use crate::style::space;

gpui_kit::actions!(player_menu, [Activate]);

pub(super) fn init(cx: &mut gpui_kit::App) {
    // Popover's Enter binding otherwise shadows the native button activation.
    cx.bind_keys([
        gpui_kit::KeyBinding::new("enter", Activate, Some("PlayerMenuControl")),
        gpui_kit::KeyBinding::new("space", Activate, Some("PlayerMenuControl")),
    ]);
}

/// Volume change per arrow key; Page Up/Down move twice as far.
const VOLUME_STEP: f32 = 5.0;

impl PlayerApp {
    pub(super) fn play_enabled(&self) -> bool {
        self.state.can_play() && matches!(self.pending, None | Some(Pending::Pause))
    }

    fn send_transport(&mut self, command: Command, pending: Pending) -> bool {
        let id = self.transport_seq + 1;
        if !self.send(Command::Transport {
            id,
            command: Box::new(command),
        }) {
            return false;
        }
        self.transport_seq = id;
        self.pending = Some(pending);
        // Any other command retires the confirmed-add line.
        self.added = None;
        true
    }

    pub(super) fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.input_text(cx);
        self.play_url(url, window, cx);
    }

    pub(super) fn play_url(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.play_enabled() {
            return;
        }
        if self.send_transport(Command::Play(url), Pending::Play) {
            // Keep typing ready for the next link; native buttons also preserve input focus.
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() || !self.state.can_next() {
            return;
        }
        let before = self.controls_enabled();
        if self.send_transport(Command::Next, Pending::Next) {
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    fn leave_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.pending, Some(Pending::Leave)) || !self.state.can_leave_voice() {
            return;
        }
        if self.send_transport(Command::LeaveVoice, Pending::Leave) {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
        }
    }

    pub(super) fn leave_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("leave-voice")
            .label("Leave voice")
            .accessibility_label("Leave voice")
            .ghost()
            .small()
            .flex_none()
            .disabled(matches!(self.pending, Some(Pending::Leave)) || !self.state.can_leave_voice())
            .help("Stop playback and leave Discord voice. Keeps the queue; Play now or Play Queue reconnects.", &self.help)
            .on_click(cx.listener(|this, _, window, cx| this.leave_voice(window, cx)))
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.controls_enabled();
        if self.controls_enabled()[2] && self.send_transport(Command::Stop, Pending::Stop) {
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    /// Sends Pause or Resume without guessing the outcome: the button keeps its label
    /// (and is disabled against repeat clicks) until the controller reports the real state.
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let command = if self.state.can_pause() {
            Command::Pause
        } else if self.state.can_resume() {
            Command::Resume
        } else {
            return;
        };
        if self.send_transport(command, Pending::Pause) {
            cx.notify();
        }
    }

    /// Hands the newest slider value to the controller. The `watch` channel keeps only the
    /// latest one, so dragging never queues a stream of commands.
    pub(super) fn request_volume(&mut self, value: f32, commit: bool, cx: &mut Context<Self>) {
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
    pub(super) fn sync_volume(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn controls_enabled(&self) -> [bool; 3] {
        [
            self.pending.is_none() && (self.state.can_pause() || self.state.can_resume()),
            self.pending.is_none() && self.state.can_next(),
            !matches!(self.pending, Some(Pending::Stop | Pending::Leave))
                && (self.state.can_stop()
                    || matches!(self.pending, Some(Pending::Play | Pending::Next))),
        ]
    }

    /// A focused button that becomes disabled would keep focus and block Tab, so focus
    /// moves back to the URL field instead.
    pub(super) fn rescue_focus(
        &self,
        before: [bool; 3],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lost = before
            .iter()
            .zip(self.controls_enabled())
            .any(|(was, now)| *was && !now);
        if lost && self.controls_focus.contains_focused(window, cx) {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let resume = self.state.phase == Phase::Paused;
        let toggle_enabled = self.controls_enabled()[0];
        let next_enabled = self.controls_enabled()[1];
        div()
            .flex()
            .flex_none()
            .gap(space::GROUP)
            .track_focus(&self.controls_focus)
            .tab_stop(false)
            .child(
                Button::new("pause-resume")
                    .when(toggle_enabled, ButtonVariants::primary)
                    .icon(if resume {
                        SpeakerIcon::Play
                    } else {
                        SpeakerIcon::Pause
                    })
                    .label(if resume { "Resume" } else { "Pause" })
                    .w(px(96.))
                    .disabled(!toggle_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(
                Button::new("next")
                    .when(self.state.next_label() == "Next", |button| {
                        button.icon(SpeakerIcon::SkipForward)
                    })
                    .label(self.state.next_label())
                    .w(px(112.))
                    .help(
                        if self.state.queue.is_empty() {
                            "Nothing is queued"
                        } else if self.state.next_label() == "Play Queue" {
                            "Start the first pending track"
                        } else {
                            "Skip to the next queued track; bypass repeat"
                        },
                        &self.help,
                    )
                    .disabled(!next_enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
            )
            .child(
                Button::new("stop")
                    .ghost()
                    .icon(SpeakerIcon::Square)
                    .accessibility_label("Stop")
                    .help("Stop playback; keep the queue", &self.help)
                    .disabled(!self.controls_enabled()[2])
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
    }

    pub(super) fn play_queue_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("next")
            .primary()
            .label("Play Queue")
            .icon(SpeakerIcon::Play)
            .disabled(!self.controls_enabled()[1])
            .help("Start the first pending track", &self.help)
            .on_click(cx.listener(|this, _, window, cx| this.next(window, cx)))
    }

    pub(super) fn repeat_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("repeat")
            .key_context("PlayerMenuControl")
            .on_action(cx.listener(|this, _: &Activate, _, _| {
                if this.state.available {
                    this.send(Command::Repeat(this.state.preferences.repeat.next()));
                }
            }))
            .label(self.state.preferences.repeat.label())
            .child(div().flex_1())
            .accessibility_label(self.state.preferences.repeat.label())
            .ghost()
            .small()
            .w_full()
            .disabled(!self.state.available)
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::Repeat(this.state.preferences.repeat.next()));
            }))
    }

    pub(super) fn remember_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("remember-queue")
            .key_context("PlayerMenuControl")
            .on_action(cx.listener(|this, _: &Activate, _, _| {
                if this.state.available {
                    this.send(Command::RememberQueue(
                        !this.state.preferences.remember_queue,
                    ));
                }
            }))
            .label(if self.state.preferences.remember_queue {
                "Remember queue: On"
            } else {
                "Remember queue: Off"
            })
            .child(div().flex_1())
            .ghost()
            .small()
            .w_full()
            .disabled(!self.state.available)
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::RememberQueue(
                    !this.state.preferences.remember_queue,
                ));
            }))
    }

    pub(super) fn queue_options(&self, cx: &Context<Self>) -> impl IntoElement {
        let repeat = self.state.preferences.repeat;
        Popover::new("queue-options-popover")
            .open(self.queue_options_open)
            .on_open_change(cx.listener(|this, open, _, cx| { this.queue_options_open = *open; this.help.dismiss(cx); cx.notify(); }))
            .anchor(gpui_kit::Anchor::TopRight)
            .trigger(Button::new("queue-options")
                .label("Queue options")
                .ghost().small().dropdown_caret(true)
                .when(repeat != RepeatMode::Off, |button| button.text_color(cx.theme().primary))

                .accessibility_label(format!("Queue options. {}. Remember queue, continue on error, clear queue", repeat.label())))
            .child(div().id("queue-options-menu").flex().flex_col().w(px(300.)).max_h(px(392.)).overflow_y_scroll().gap_1()
                .child(self.repeat_button(cx))
                .child(menu_detail("After a track finishes. Next and Stop never repeat.", cx))
                .child(self.remember_button(cx))
                .child(menu_detail("Restore pending tracks on launch. Turning off erases only the saved queue.", cx))
                .child(Button::new("continue-on-error")
                    .key_context("PlayerMenuControl")
                    .on_action(cx.listener(|this, _: &Activate, _, _| { if this.state.available { this.send(Command::ContinueOnError(!this.state.preferences.continue_on_error)); } }))
                    .label(if self.state.preferences.continue_on_error { "Continue on error: On" } else { "Continue on error: Off" })
                    .child(div().flex_1())
                    .ghost().small().w_full().disabled(!self.state.available)

                    .on_click(cx.listener(|this, _, _, _| {
                        this.send(Command::ContinueOnError(!this.state.preferences.continue_on_error));
                    })))
                .child(menu_detail("Skip unplayable queued tracks. Stop after 3 in a row.", cx))
                .child(div().mt_2().pt_2().border_t_1().border_color(crate::style::separator())
                    .child(Button::new("queue-clear")
                        .key_context("PlayerMenuControl")
                        .on_action(cx.listener(|this, _: &Activate, _, _| { if this.state.controller_ready && (!this.state.queue.is_empty() || this.state.import == ImportStatus::Importing) { this.send(Command::ClearQueue); } })).label("Clear queue").child(div().flex_1()).ghost().small().w_full()
                        .disabled(!self.state.controller_ready || (self.state.queue.is_empty() && self.state.import != ImportStatus::Importing))

                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::ClearQueue); }))))
                .child(menu_detail("Keeps the current track; cancels playlist imports.", cx))
                .child(div().pt_2().px_2().text_xs().text_color(cx.theme().muted_foreground)
                    .child("Preferences save automatically.")).test_support())
    }

    fn import_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_pending {
            return;
        }
        let running = self.state.import == ImportStatus::Importing;
        if !running && !self.state.can_import() {
            return;
        }
        let command = if running {
            Command::CancelImport
        } else {
            Command::ImportPlaylist(self.input_text(cx))
        };
        if self.send(command) {
            self.import_pending = true;
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
        }
    }

    /// Import playlist, or Cancel import while one runs. A playlist link in the field gives it
    /// the accent; otherwise it stays a quiet action beside Add to queue.
    pub(super) fn import_button(
        &self,
        playlist_link: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let running = self.state.import == ImportStatus::Importing;
        let label = if running {
            "Cancel import"
        } else {
            "Import playlist"
        };
        div()
            .flex_none()
            .track_focus(&self.import_focus)
            .tab_stop(false)
            .child(
                Button::new("import-playlist")
                    .key_context("PlayerMenuControl")
                    .on_action(cx.listener(|this, _: &Activate, window, cx| {
                        this.import_playlist(window, cx);
                    }))
                    .icon(if running {
                        SpeakerIcon::Close
                    } else {
                        SpeakerIcon::Import
                    })
                    .label(label)
                    .accessibility_label(label)
                    .help(
                        if running {
                            "Stop the playlist import; tracks already found stay queued."
                        } else {
                            "Add the playlist in the link to the queue, up to 100 tracks. Never interrupts playback."
                        },
                        &self.help,
                    )
                    .ghost()
                    .small()
                    .when(playlist_link && !running, |button| {
                        button.text_color(cx.theme().primary)
                    })
                    .disabled(self.import_pending || (!running && !self.state.can_import()))
                    .on_click(cx.listener(|this, _, window, cx| this.import_playlist(window, cx))),
            )
    }

    pub(super) fn volume_row(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
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
            .flex_none()
            .items_center()
            .gap(space::TIGHT)
            .child(
                Icon::new(speaker)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .id("volume")
                    .role(gpui_kit::Role::Group)
                    .aria_label("Volume. Arrow keys: 5 percent; Page Up/Down: 10 percent; Home: mute; End: full volume.")
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .rounded(theme.radius)
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
                    .w(px(40.))
                    .flex_none()
                    .text_right()
                    .text_sm()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(if available {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(format!("{percent}%"))
                    .test_support(),
            )
    }
}

fn menu_detail(text: &'static str, cx: &Context<PlayerApp>) -> impl IntoElement {
    div()
        .px_2()
        .pb_1()
        .text_xs()
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
}
