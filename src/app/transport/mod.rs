//! Transport commands and the window's primary controls.

use gpui_kit::assets::IconName as SpeakerIcon;
use gpui_kit::component::{Disableable, Sizable, button::ButtonVariants};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt,
    Window, div, prelude::FluentBuilder, px,
};

use crate::player::{Command, Phase};
use crate::style::space;

use super::PlayerApp;
use super::composer::LinkKind;
use super::ui::help::HelpExt;

#[derive(Clone, Copy)]
pub(super) enum Pending {
    Play,
    Next,
    Pause,
    Stop,
    Leave,
}

impl PlayerApp {
    pub(super) fn play_enabled(&self) -> bool {
        self.state.can_play() && matches!(self.pending, None | Some(Pending::Pause))
    }

    pub(super) fn send_transport(&mut self, command: Command, pending: Pending) -> bool {
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
        if LinkKind::of(&url) == LinkKind::Query {
            self.submit_search(window, cx);
            return;
        }
        if url.trim().is_empty() {
            self.local_hint = Some("Paste a YouTube link to play.".into());
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
            return;
        }
        self.local_hint = None;
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
        self.focus
            .button("leave-voice")
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
                self.focus
                    .button("pause-resume")
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
                self.focus
                    .button("next")
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
                self.focus
                    .button("stop")
                    .ghost()
                    .icon(SpeakerIcon::Square)
                    .accessibility_label("Stop")
                    .help("Stop playback; keep the queue", &self.help)
                    .disabled(!self.controls_enabled()[2])
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
    }

    pub(super) fn play_queue_button(&self, cx: &Context<Self>) -> impl IntoElement {
        self.focus
            .button("next")
            .primary()
            .label("Play Queue")
            .icon(SpeakerIcon::Play)
            .disabled(!self.controls_enabled()[1])
            .help("Start the first pending track", &self.help)
            .on_click(cx.listener(|this, _, window, cx| this.next(window, cx)))
    }

    pub(super) fn audio_footer(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("audio-footer")
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(space::GROUP)
            .mt(space::GROUP)
            .pt(space::TIGHT)
            .border_t_1()
            .border_color(crate::style::separator())
            .child(self.queue_options(cx))
            .child(
                div()
                    .w(px(220.))
                    .flex_none()
                    .child(self.volume_row(window, cx)),
            )
            .test_support()
    }
}
