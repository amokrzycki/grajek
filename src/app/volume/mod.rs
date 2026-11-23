//! Bot volume: the slider, its keyboard model and controller-confirmed sync.

use gpui_kit::component::{ActiveTheme, Icon, Sizable, slider::Slider};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div,
};

use crate::player::{VolumeRequest, percent_from};
use crate::preferences::VOLUME_MAX;
use crate::style::space;

use super::PlayerApp;

/// Volume change per arrow key; Page Up/Down move twice as far.
const VOLUME_STEP: f32 = 5.0;

impl PlayerApp {
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
        let max = f32::from(VOLUME_MAX);
        let target = match key {
            "left" | "down" => current - VOLUME_STEP,
            "right" | "up" => current + VOLUME_STEP,
            "pagedown" => current - 2.0 * VOLUME_STEP,
            "pageup" => current + 2.0 * VOLUME_STEP,
            "home" => 0.0,
            "end" => max,
            _ => return,
        };
        let target = target.clamp(0.0, max);
        self.volume
            .update(cx, |slider, cx| slider.set_value(target, window, cx));
        self.request_volume(target, true, cx);
    }

    pub(super) fn volume_row(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let available = self.state.available;
        let percent = percent_from(self.slider_value(cx)).unwrap_or(self.state.volume);
        let focused = self.focus.keyboard && self.volume_focus.is_focused(window);
        let speaker = if percent == 0 {
            gpui_kit::assets::IconName::VolumeX
        } else {
            gpui_kit::assets::IconName::Volume2
        };
        // A tracked handle decides its own tab order: reachable by Tab only while usable.
        let focus = self.volume_focus.clone().tab_stop(available);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(space::TIGHT)
            .child(Icon::new(speaker).small().text_color(theme.muted_foreground))
            .child(div().text_xs().text_color(theme.muted_foreground).child("Bot volume"))
            .child(
                div()
                    .id("volume")
                    .role(gpui_kit::Role::Group)
                    .aria_label(format!(
                        "Bot volume. Arrow keys: {VOLUME_STEP} percent; Page Up/Down: {} percent; Home: mute; End: full volume.",
                        VOLUME_STEP * 2.0
                    ))
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
                    .child(Slider::new(&self.volume).disabled(!available))
                    .test_support(),
            )
            .child(
                div()
                    .id("volume-percent")
                    .w(gpui_kit::px(40.))
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
