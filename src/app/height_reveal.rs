//! View-clocked height reveals for Settings disclosure panels. The clock lives
//! on the view, not in element state: GPUI drops element state when Settings is
//! swapped away, so a keyed kit transition would replay a finished reveal on
//! return. Frames are requested only while a reveal runs; reduced motion jumps
//! straight to the final height and requests nothing.
use std::time::{Duration, Instant};

use super::motion::Pulse;

use gpui_kit::base::MotionReveal;
use gpui_kit::base::animation::ease_out_cubic;
use gpui_kit::{AnyElement, App, IntoElement, Window};

pub(super) const REVEAL: Duration = Duration::from_millis(180);

#[derive(Default)]
struct Slot {
    seen: Option<bool>,
    value: f32,
    pulse: Option<(f32, f32, Pulse)>,
}

impl Slot {
    fn sample(&mut self, open: bool, now: Instant) -> f32 {
        if self.seen != Some(open) {
            let adopted = self.seen.is_none();
            self.seen = Some(open);
            if adopted {
                self.value = f32::from(open);
                self.pulse = None;
            } else {
                self.pulse = Some((self.value, f32::from(open), Pulse { started: now }));
            }
        }
        if let Some((from, target, pulse)) = self.pulse {
            if let Some(step) = pulse.progress(now, REVEAL) {
                self.value = from + (target - from) * ease_out_cubic(step);
            } else {
                self.pulse = None;
                self.value = target;
            }
        } else {
            self.value = f32::from(open);
        }
        self.value
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct RevealFrame {
    pub(super) credentials: f32,
    pub(super) preferences: f32,
    pub(super) guide: f32,
    animated: bool,
}

impl RevealFrame {
    /// Reduced motion mounts the content bare: the kit reveal asks for a frame
    /// to measure it.
    pub(super) fn panel(
        self,
        id: &'static str,
        progress: f32,
        open: bool,
        content: impl FnOnce() -> AnyElement,
    ) -> Option<AnyElement> {
        if !self.animated {
            return open.then(content);
        }
        (open || progress > 0.0)
            .then(|| MotionReveal::new(id, progress, content()).into_any_element())
    }
}

#[derive(Default)]
pub(super) struct HeightReveals {
    credentials: Slot,
    preferences: Slot,
    guide: Slot,
}

impl HeightReveals {
    pub(super) fn sample(
        &mut self,
        credentials: bool,
        preferences: bool,
        guide: bool,
        window: &Window,
        cx: &App,
    ) -> RevealFrame {
        if cx.reduce_motion() {
            *self = Self::default();
            return RevealFrame {
                credentials: f32::from(credentials),
                preferences: f32::from(preferences),
                guide: f32::from(guide),
                animated: false,
            };
        }
        let now = cx.background_executor().now();
        let frame = RevealFrame {
            credentials: self.credentials.sample(credentials, now),
            preferences: self.preferences.sample(preferences, now),
            guide: self.guide.sample(guide, now),
            animated: true,
        };
        if self.running() {
            window.request_animation_frame();
        }
        frame
    }

    fn running(&self) -> bool {
        self.credentials.pulse.is_some()
            || self.preferences.pulse.is_some()
            || self.guide.pulse.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reversal_mid_reveal_restarts_from_the_current_value() {
        let start = Instant::now();
        let mut slot = Slot::default();
        slot.sample(false, start);
        slot.sample(true, start);
        let half = slot.sample(true, start + REVEAL / 2);
        assert!((0.0..1.0).contains(&half), "mid-reveal value, got {half}");

        let reversed = slot.sample(false, start + REVEAL / 2);
        assert!(
            (reversed - half).abs() < f32::EPSILON,
            "the reversal begins where the reveal was, got {reversed} from {half}"
        );
        let later = slot.sample(false, start + REVEAL / 2 + REVEAL / 4);
        assert!(later < half, "the collapse heads back down, got {later}");
        assert!(slot.sample(false, start + REVEAL * 2).abs() < f32::EPSILON);
    }
}
