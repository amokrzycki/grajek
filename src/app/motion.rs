//! Controller-confirmed colour pulses. The clock lives on the window, not in element
//! state: GPUI drops element state when a virtualized row scrolls away or Settings
//! replaces the player, so element-keyed one-shots would replay on return. Frames are
//! requested only while a pulse runs; reduced motion jumps straight to the resting
//! values and requests nothing.
use std::time::{Duration, Instant};

use gpui_kit::base::animation::{Lerp, ease_out_cubic};
use gpui_kit::{App, Hsla, Window};

use super::PlayerApp;
use crate::player::State;

pub(super) const ARRIVAL: Duration = Duration::from_millis(200);
pub(super) const MARK: Duration = Duration::from_millis(240);
pub(super) const SAVED: Duration = Duration::from_millis(200);
pub(super) const CALLOUT: Duration = Duration::from_millis(150);
/// A move request the controller has not answered within this long is forgotten.
const SHIFT_WAIT: Duration = Duration::from_secs(2);
const MARK_PEAK: f32 = 0.16;

#[derive(Clone, Copy)]
struct Pulse {
    started: Instant,
}

impl Pulse {
    /// `None` once the pulse has finished.
    fn progress(self, now: Instant, length: Duration) -> Option<f32> {
        let elapsed = now.saturating_duration_since(self.started);
        (elapsed < length).then(|| elapsed.as_secs_f32() / length.as_secs_f32())
    }
}

/// What this frame should look like. `Default` is the resting presentation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Frame {
    arrival: Option<(u64, f32)>,
    mark: Option<(u64, f32)>,
    saved: Option<(String, f32)>,
    error: Option<f32>,
    notice: Option<f32>,
}

impl Frame {
    pub(super) fn settle(&self, track_id: u64, from: Hsla, rest: Hsla) -> Hsla {
        match self.arrival {
            Some((key, progress)) if key == track_id => from.lerp(&rest, ease_out_cubic(progress)),
            _ => rest,
        }
    }

    /// The wash for a queue entry: brass fading to nothing, `None` at rest.
    pub(super) fn wash(&self, entry: u64, brass: Hsla) -> Option<Hsla> {
        match self.mark {
            Some((key, progress)) if key == entry => {
                Some(brass.opacity(MARK_PEAK * (1.0 - ease_out_cubic(progress))))
            }
            _ => None,
        }
    }

    pub(super) fn saved(&self, url: &str, from: Hsla, rest: Hsla) -> Hsla {
        match &self.saved {
            Some((key, progress)) if key == url => from.lerp(&rest, ease_out_cubic(*progress)),
            _ => rest,
        }
    }

    pub(super) fn error_fade(&self) -> Option<f32> {
        self.error.map(ease_out_cubic)
    }

    pub(super) fn notice_fade(&self) -> Option<f32> {
        self.notice.map(ease_out_cubic)
    }
}

#[derive(Default)]
pub(super) struct Motion {
    arrival: Option<(u64, Pulse)>,
    mark: Option<(u64, Pulse)>,
    saved: Option<(String, Pulse)>,
    error: Option<Pulse>,
    notice: Option<Pulse>,
    /// The entry the user asked to move, its position then, and when.
    shift: Option<(u64, usize, Instant)>,
}

impl Motion {
    /// Called with every controller snapshot, before it replaces the previous one.
    pub(super) fn observe(&mut self, previous: &State, next: &State, now: Instant) {
        let playing = |state: &State| state.visible_track().is_some();
        if playing(next) && !(playing(previous) && previous.track_id == next.track_id) {
            self.arrival = Some((next.track_id, Pulse { started: now }));
        } else if !playing(next) || self.arrival.is_some_and(|(key, _)| key != next.track_id) {
            self.arrival = None;
        }
        if let Some((id, position, asked)) = self.shift {
            if now.saturating_duration_since(asked) >= SHIFT_WAIT {
                self.shift = None;
            } else {
                match next.queue.iter().position(|item| item.id == id) {
                    Some(now_at) if now_at != position => {
                        self.mark = Some((id, Pulse { started: now }));
                        self.shift = None;
                    }
                    Some(_) => {}
                    None => self.shift = None,
                }
            }
        }
        if let Some((key, _)) = self.mark
            && !next.queue.iter().any(|item| item.id == key)
        {
            self.mark = None;
        }
    }

    pub(super) fn added(&mut self, entry: u64, now: Instant) {
        self.mark = Some((entry, Pulse { started: now }));
    }

    /// Only a snapshot that shows `entry` away from `position` produces a mark.
    pub(super) fn asked_to_move(&mut self, entry: u64, position: usize, now: Instant) {
        self.shift = Some((entry, position, now));
    }

    pub(super) fn saved(&mut self, url: String, now: Instant) {
        self.saved = Some((url, Pulse { started: now }));
    }

    pub(super) fn error_shown(&mut self, now: Instant) {
        self.error = Some(Pulse { started: now });
    }

    pub(super) fn notice_shown(&mut self, now: Instant) {
        self.notice = Some(Pulse { started: now });
    }

    /// (arrival, mark) identities of the running pulses.
    #[cfg(test)]
    pub(super) fn keys(&self) -> (Option<u64>, Option<u64>) {
        (
            self.arrival.map(|(key, _)| key),
            self.mark.map(|(key, _)| key),
        )
    }

    /// Samples the clock and asks for the next frame only while a pulse runs.
    pub(super) fn frame(&mut self, window: &Window, cx: &App) -> Frame {
        if cx.reduce_motion() {
            *self = Self::default();
            return Frame::default();
        }
        let now = cx.background_executor().now();
        let saved = self.saved.as_ref().and_then(|(url, pulse)| {
            pulse
                .progress(now, SAVED)
                .map(|progress| (url.clone(), progress))
        });
        if saved.is_none() {
            self.saved = None;
        }
        let frame = Frame {
            arrival: sample_keyed(&mut self.arrival, now, ARRIVAL),
            mark: sample_keyed(&mut self.mark, now, MARK),
            saved,
            error: sample(&mut self.error, now, CALLOUT),
            notice: sample(&mut self.notice, now, CALLOUT),
        };
        if frame != Frame::default() {
            window.request_animation_frame();
        }
        frame
    }
}

fn sample_keyed<K: Copy>(
    slot: &mut Option<(K, Pulse)>,
    now: Instant,
    length: Duration,
) -> Option<(K, f32)> {
    let (key, pulse) = (*slot)?;
    let progress = pulse.progress(now, length);
    if progress.is_none() {
        *slot = None;
    }
    progress.map(|progress| (key, progress))
}

fn sample(slot: &mut Option<Pulse>, now: Instant, length: Duration) -> Option<f32> {
    let pulse = (*slot)?;
    let progress = pulse.progress(now, length);
    if progress.is_none() {
        *slot = None;
    }
    progress
}

impl PlayerApp {
    pub(super) fn observe_callouts(&mut self, next: &State, now: Instant) {
        let error = displayed_error(next);
        if error != self.shown_error {
            if error.is_some() {
                self.motion.error_shown(now);
            }
            self.shown_error = error;
        }
        let notice = next.notice.clone();
        if notice != self.shown_notice {
            if notice.is_some() {
                self.motion.notice_shown(now);
            }
            self.shown_notice = notice;
        }
    }
}

pub(super) fn displayed_error(state: &State) -> Option<String> {
    if let Some(error) = &state.error {
        return Some(error.clone());
    }
    match &state.seek {
        crate::player::SeekStatus::Failed(message) => Some(message.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn pulses_finish_and_never_overshoot() {
        let start = Instant::now();
        let pulse = Pulse { started: start };
        assert_eq!(pulse.progress(start, ARRIVAL), Some(0.0));
        assert!(
            pulse
                .progress(at(start, 100), ARRIVAL)
                .is_some_and(|p| p > 0.0 && p < 1.0)
        );
        assert_eq!(pulse.progress(at(start, 200), ARRIVAL), None);
        let samples = [0.0_f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
        assert!(
            samples
                .iter()
                .all(|&t| (0.0..=1.0).contains(&ease_out_cubic(t)))
        );
        assert!(
            samples
                .windows(2)
                .all(|pair| ease_out_cubic(pair[0]) < ease_out_cubic(pair[1])),
            "monotonic: no overshoot or bounce"
        );
    }

    #[test]
    fn settle_applies_only_to_the_arriving_track() {
        let from = gpui_kit::hsla(0.0, 0.0, 0.2, 1.0);
        let rest = gpui_kit::hsla(0.0, 0.0, 0.9, 1.0);
        let frame = Frame {
            arrival: Some((7, 0.0)),
            ..Frame::default()
        };
        assert_eq!(frame.settle(7, from, rest), from);
        assert_eq!(frame.settle(8, from, rest), rest, "another track rests");
        assert_eq!(Frame::default().settle(7, from, rest), rest);
    }

    #[test]
    fn wash_fades_to_nothing_for_one_entry_only() {
        let brass = gpui_kit::hsla(0.1, 0.5, 0.7, 1.0);
        let frame = Frame {
            mark: Some((3, 0.0)),
            ..Frame::default()
        };
        assert!(frame.wash(3, brass).is_some_and(|wash| wash.a > 0.1));
        assert!(frame.wash(4, brass).is_none());
        let late = Frame {
            mark: Some((3, 0.999)),
            ..Frame::default()
        };
        assert!(late.wash(3, brass).is_some_and(|wash| wash.a < 0.01));
    }

    #[test]
    fn saved_settles_only_for_the_confirmed_url() {
        let from = gpui_kit::hsla(0.0, 0.0, 0.2, 1.0);
        let rest = gpui_kit::hsla(0.0, 0.0, 0.9, 1.0);
        let frame = Frame {
            saved: Some(("https://youtu.be/a".into(), 0.0)),
            ..Frame::default()
        };
        assert_eq!(frame.saved("https://youtu.be/a", from, rest), from);
        assert_eq!(frame.saved("https://youtu.be/b", from, rest), rest);
    }
}
