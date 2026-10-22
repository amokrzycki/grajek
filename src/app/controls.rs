use gpui_kit::assets::IconName as SpeakerIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    slider::Slider,
};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Styled,
    TestSupportExt, Window, div, prelude::FluentBuilder, px,
};

use super::{Pending, PlayerApp};
use crate::player::{Command, Phase, RepeatMode, VolumeRequest, percent_from};

/// Volume change per arrow key; Page Up/Down move twice as far.
const VOLUME_STEP: f32 = 5.0;

impl PlayerApp {
    pub(super) fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.input_text(cx);
        self.play_url(url, window, cx);
    }

    pub(super) fn play_url(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() || !self.state.can_play() {
            return;
        }
        let before = self.controls_enabled();
        if self.send(Command::Play(url)) {
            self.pending = Some(Pending::Play);
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() || !self.state.can_next() {
            return;
        }
        let before = self.controls_enabled();
        if self.send(Command::Next) {
            self.pending = Some(Pending::Next);
            self.rescue_focus(before, window, cx);
            cx.notify();
        }
    }

    fn leave_voice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.pending, Some(Pending::Leave)) || !self.state.can_leave_voice() {
            return;
        }
        if self.send(Command::LeaveVoice) {
            self.pending = Some(Pending::Leave);
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
        }
    }

    pub(super) fn leave_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("leave-voice")
            .label("Leave voice")
            .accessibility_label("Leave voice")
            .small()
            .flex_none()
            .disabled(matches!(self.pending, Some(Pending::Leave)) || !self.state.can_leave_voice())
            .tooltip("Stop playback and leave Discord voice. Keeps the queue; Play URL or Play Queue reconnects.")
            .on_click(cx.listener(|this, _, window, cx| this.leave_voice(window, cx)))
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.controls_enabled();
        if self.controls_enabled()[2] && self.send(Command::Stop) {
            self.pending = Some(Pending::Stop);
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
        if self.send(command) {
            self.pending = Some(Pending::Pause);
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
            .gap_2()
            .track_focus(&self.controls_focus)
            .tab_stop(false)
            .child(
                Button::new("pause-resume")
                    .when(resume, ButtonVariants::primary)
                    .icon(if resume {
                        SpeakerIcon::Play
                    } else {
                        SpeakerIcon::Pause
                    })
                    .label(if resume { "Resume" } else { "Pause" })
                    .flex_1()
                    .disabled(!toggle_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(
                Button::new("next")
                    .icon(SpeakerIcon::SkipForward)
                    .label(self.state.next_label())
                    .w(px(116.))
                    .tooltip(if self.state.queue.is_empty() {
                        "Nothing is queued"
                    } else if self.state.next_label() == "Play Queue" {
                        "Start the first pending track"
                    } else {
                        "Skip to the next queued track; bypass repeat"
                    })
                    .disabled(!next_enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
            )
            .child(
                Button::new("stop")
                    .ghost()
                    .label("Stop")
                    .w(px(64.))
                    .disabled(!self.controls_enabled()[2])
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
    }

    pub(super) fn repeat_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("repeat")
            .label(self.state.preferences.repeat.label())
            .accessibility_label(self.state.preferences.repeat.label())
            .when(self.state.preferences.repeat != RepeatMode::Off, ButtonVariants::primary)
            .h(px(28.))
            .w(px(120.))
            .disabled(!self.state.available)
            .tooltip("Change repeat: Off → Repeat Track → Repeat Queue. Successful natural completion repeats; Next and Stop bypass repeat.")
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::Repeat(this.state.preferences.repeat.next()));
            }))
    }

    pub(super) fn remember_button(&self, cx: &Context<Self>) -> impl IntoElement {
        Button::new("remember-queue")
            .label(if self.state.preferences.remember_queue { "Remember queue: On" } else { "Remember queue: Off" })
            .ghost()
            .xsmall()
            .disabled(!self.state.available)
            .tooltip("Save pending entries across restarts without starting playback. Turning off removes saved entries and keeps this session’s queue.")
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::RememberQueue(!this.state.preferences.remember_queue));
            }))
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
                    .w(px(48.))
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
