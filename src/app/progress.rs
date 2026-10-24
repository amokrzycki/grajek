use gpui_kit::component::{
    ActiveTheme,
    slider::{Slider, SliderEvent},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AccessibleAction, App, Context, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div,
};
use std::time::Duration;

use super::{PlayerApp, clock};
use crate::player::{Command, SeekStatus, seek_target};

impl PlayerApp {
    pub(super) fn progress_accessibility(
        &self,
        action: AccessibleAction,
        data: Option<&gpui_kit::accesskit::ActionData>,
        cx: &mut Context<Self>,
    ) {
        if !self.state.can_seek() {
            return;
        }
        match action {
            AccessibleAction::Increment => {
                self.progress_key("right", cx);
            }
            AccessibleAction::Decrement => {
                self.progress_key("left", cx);
            }
            AccessibleAction::SetValue => {
                if let Some(gpui_kit::accesskit::ActionData::NumericValue(seconds)) = data
                    && let Ok(position) = Duration::try_from_secs_f64(*seconds)
                    && let Some(position) = self
                        .state
                        .track
                        .as_ref()
                        .and_then(|track| seek_target(position, track.duration))
                {
                    self.send(Command::Seek {
                        track_id: self.state.track_id,
                        position,
                    });
                }
            }
            _ => {}
        }
    }

    fn progress_accessibility_listener(
        action: AccessibleAction,
        cx: &Context<Self>,
    ) -> impl FnMut(Option<&gpui_kit::accesskit::ActionData>, &mut Window, &mut App) + use<> {
        let entity = cx.entity().downgrade();
        move |data, _, cx| {
            let _ = entity.update(cx, |this, cx| this.progress_accessibility(action, data, cx));
        }
    }
    fn progress_target(&self, value: f32) -> Option<Duration> {
        let duration = self.state.visible_track()?.duration?;
        if !value.is_finite() {
            return None;
        }
        let seconds = duration.as_secs_f64() * f64::from(value.clamp(0., 1000.)) / 1000.;
        seek_target(Duration::try_from_secs_f64(seconds).ok()?, Some(duration))
    }

    pub(super) fn progress_event(
        &mut self,
        event: &SliderEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SliderEvent::Change(value) => {
                let id = *self.progress_gesture.get_or_insert(self.state.track_id);
                if id == self.state.track_id && self.state.can_seek() {
                    self.progress_preview = self.progress_target(value.end());
                } else {
                    self.progress_preview = None;
                    self.sync_progress(window, cx);
                }
            }
            SliderEvent::Release(value) => {
                let id = self.progress_gesture.take();
                self.progress_preview = None;
                if id == Some(self.state.track_id)
                    && self.state.can_seek()
                    && let Some(position) = self.progress_target(value.end())
                {
                    self.send(Command::Seek {
                        track_id: self.state.track_id,
                        position,
                    });
                }
                // Release always returns to confirmed audio position until the controller answers.
                self.sync_progress(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn sync_progress(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.progress_preview.is_some() {
            return;
        }
        let value = self
            .state
            .visible_track()
            .and_then(|track| track.duration)
            .filter(|duration| !duration.is_zero())
            .map_or(0., |duration| {
                (self.state.position.as_secs_f64() / duration.as_secs_f64()).clamp(0., 1.) * 1000.
            });
        #[expect(
            clippy::cast_possible_truncation,
            reason = "normalized slider range is 0..=1000"
        )]
        let value = value as f32;
        if (self.progress.read(cx).value().end() - value).abs() > f32::EPSILON {
            self.progress
                .update(cx, |slider, cx| slider.set_value(value, window, cx));
        }
    }

    fn progress_key(&self, key: &str, cx: &mut Context<Self>) -> bool {
        if !self.state.can_seek() {
            return false;
        }
        let current = match self.state.seek {
            SeekStatus::Pending(target) => target,
            _ => self.state.position,
        };
        let target = match key {
            "left" => current.saturating_sub(Duration::from_secs(5)),
            "right" => current.saturating_add(Duration::from_secs(5)),
            "pagedown" => current.saturating_sub(Duration::from_secs(10)),
            "pageup" => current.saturating_add(Duration::from_secs(10)),
            "home" => Duration::ZERO,
            "end" => Duration::MAX,
            _ => return false,
        };
        if let Some(position) = self
            .state
            .track
            .as_ref()
            .and_then(|track| seek_target(target, track.duration))
        {
            self.send(Command::Seek {
                track_id: self.state.track_id,
                position,
            });
            cx.notify();
        }
        true
    }

    pub(super) fn progress_status(&self) -> String {
        if let Some(target) = self.progress_preview {
            format!("Preview {}", clock(target))
        } else if matches!(self.state.seek, SeekStatus::Pending(_)) {
            "Seeking…".into()
        } else {
            self.state.phase.label().into()
        }
    }

    pub(super) fn progress_row(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let visible = self
            .state
            .visible_track()
            .and_then(|track| track.duration)
            .is_some_and(|duration| !duration.is_zero());
        let enabled = self.state.can_seek();
        let focus = self.progress_focus.clone().tab_stop(enabled);
        let status = self.progress_status();
        let label = format!(
            "Playback position {}. {status} Left/Right: 5 seconds; Page Up/Down: 10 seconds; Home: start; End: one second before end.",
            clock(self.state.position)
        );
        div().flex_none().when(visible, |row| {
            row.flex().items_center().gap_2().child(
                div()
                    .id("seek-slider")
                    .flex_1()
                    .min_w_0()
                    .px_1()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(if self.progress_focus.is_focused(window) {
                        theme.ring
                    } else {
                        theme.transparent
                    })
                    .track_focus(&self.progress_focus)
                    .role(gpui_kit::Role::Slider)
                    .aria_label(label)
                    .a11y_synthetic_children(move |builder| {
                        if !enabled {
                            builder.parent_node().set_disabled();
                        }
                    })
                    .on_a11y_action(
                        AccessibleAction::Increment,
                        Self::progress_accessibility_listener(AccessibleAction::Increment, cx),
                    )
                    .on_a11y_action(
                        AccessibleAction::Decrement,
                        Self::progress_accessibility_listener(AccessibleAction::Decrement, cx),
                    )
                    .on_a11y_action(
                        AccessibleAction::SetValue,
                        Self::progress_accessibility_listener(AccessibleAction::SetValue, cx),
                    )
                    .aria_numeric_value(self.state.position.as_secs_f64())
                    .aria_numeric_value_step(5.)
                    .aria_min_numeric_value(0.)
                    .aria_max_numeric_value(
                        self.state
                            .track
                            .as_ref()
                            .and_then(|track| seek_target(Duration::MAX, track.duration))
                            .unwrap_or_default()
                            .as_secs_f64(),
                    )
                    .aria_value(clock(self.state.position))
                    .capture_any_mouse_down(cx.listener(move |this, _, window, cx| {
                        if enabled {
                            this.progress_gesture = Some(this.state.track_id);
                            window.focus(&focus, cx);
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if !event.keystroke.modifiers.control
                            && !event.keystroke.modifiers.alt
                            && !event.keystroke.modifiers.platform
                            && this.progress_key(event.keystroke.key.as_str(), cx)
                        {
                            cx.stop_propagation();
                        }
                    }))
                    // Present one authoritative accessible control, not the kit's normalized inner slider.
                    .child(
                        div()
                            .id("seek-visual")
                            .role(gpui_kit::Role::Group)
                            .a11y_synthetic_children(|builder| builder.parent_node().set_hidden())
                            .child(Slider::new(&self.progress).disabled(!enabled)),
                    )
                    .test_support(),
            )
        })
    }
}
