use super::support::frame;
use super::*;
use crate::app::motion::VIEW;
use crate::app::motion::{Direction, Frame};

fn toggle(cx: &TestAppContext, view: &Entity<PlayerApp>, open: bool) {
    cx.update(|cx| {
        view.update(cx, |app, cx| {
            app.settings_open = open;
            cx.notify();
        });
    });
}

/// Production startup: `PlayerApp::new` leaves Settings open and focused.
#[expect(
    clippy::expect_used,
    reason = "GPUI window setup failures must fail the test"
)]
fn open_settings_first(
    cx: &mut TestAppContext,
    initial: State,
) -> (AnyWindowHandle, Entity<PlayerApp>) {
    let (commands, _received) = mpsc::channel(16);
    let (volume, _) = watch::channel(VolumeRequest::default());
    let (_states, updates) = watch::channel(initial);
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| PlayerApp::new(window, cx, commands, volume, updates))
        })
        .expect("test window should open")
    });
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("test window should render");
    (window, view)
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "a missing mid-pulse presentation must fail the easing assertions"
)]
fn entering_settings_slides_in_from_the_right(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    assert_eq!(
        frame(cx, window, &view).view(),
        None,
        "the player at rest has no pulse"
    );
    toggle(cx, &view, true);
    let first = frame(cx, window, &view);
    assert_eq!(first.view(), Some((Direction::Right, 0.0)));
    assert_eq!(
        first.view_shift(),
        Some((0.0, 12.0)),
        "Settings starts invisible, 12px right of its resting inset"
    );
    assert_eq!(bounds(cx, window, "discord-settings").left(), px(28.));
    cx.executor().advance_clock(VIEW / 2);
    let middle = frame(cx, window, &view);
    let (direction, progress) = middle.view().expect("the pulse is running");
    assert_eq!(direction, Direction::Right);
    assert!(progress > 0.0 && progress < 1.0);
    let (opacity, shift) = middle.view_shift().expect("mid-pulse presentation");
    assert!(opacity > 0.0 && opacity < 1.0, "fading in");
    assert!(
        shift > 0.0 && shift < 12.0,
        "settling back to the resting inset"
    );
}

#[gpui_kit::test]
fn leaving_settings_returns_the_player_from_the_left(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    let resting_input = bounds(cx, window, "url").left();
    toggle(cx, &view, true);
    drawn(cx, window);
    cx.executor().advance_clock(VIEW);
    drawn(cx, window);
    toggle(cx, &view, false);
    let first = frame(cx, window, &view);
    assert_eq!(first.view(), Some((Direction::Left, 0.0)));
    assert_eq!(first.view_shift(), Some((0.0, -12.0)));
    assert_eq!(bounds(cx, window, "url").left(), resting_input - px(12.));
    assert!(
        present(cx, window, "url"),
        "the incoming player is interactive"
    );
}

#[gpui_kit::test]
fn settings_open_at_startup_never_animates(cx: &mut TestAppContext) {
    let (window, view) = open_settings_first(cx, state(Phase::Ready));
    assert!(present(cx, window, "discord-settings"));
    assert_eq!(
        frame(cx, window, &view).view(),
        None,
        "the surface Settings opens with is already at rest"
    );
    drawn(cx, window);
    assert_eq!(frame(cx, window, &view).view(), None);
    toggle(cx, &view, false);
    assert_eq!(
        frame(cx, window, &view).view(),
        Some((Direction::Left, 0.0)),
        "a later real flip still transitions"
    );
}

#[gpui_kit::test]
fn the_view_pulse_finishes_and_the_frame_rests(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    toggle(cx, &view, true);
    drawn(cx, window);
    cx.executor().advance_clock(VIEW);
    assert_eq!(
        frame(cx, window, &view),
        Frame::default(),
        "finished means resting"
    );
    assert_eq!(
        frame(cx, window, &view),
        Frame::default(),
        "a render after the pulse is over never restarts it"
    );
    assert_eq!(bounds(cx, window, "discord-settings").left(), px(16.));
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(
        requested(cx, window),
        0,
        "a finished pulse requests no frame"
    );
}

#[gpui_kit::test]
fn reduced_motion_needs_no_pulse_and_no_frame(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    cx.update(|cx| cx.set_reduce_motion(true));
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(requested(cx, window), 0);
    toggle(cx, &view, true);
    assert_eq!(frame(cx, window, &view), Frame::default());
    assert_eq!(requested(cx, window), 0, "reduced motion requests no frame");
    assert!(present(cx, window, "discord-settings"));
}

#[gpui_kit::test]
fn a_settings_round_trip_does_not_replay_a_finished_pulse(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    toggle(cx, &view, true);
    drawn(cx, window);
    cx.executor().advance_clock(VIEW);
    drawn(cx, window);
    toggle(cx, &view, false);
    drawn(cx, window);
    cx.executor().advance_clock(VIEW);
    drawn(cx, window);
    assert_eq!(
        frame(cx, window, &view),
        Frame::default(),
        "the round trip left nothing running"
    );
    for _ in 0..3 {
        drawn(cx, window);
    }
    assert_eq!(
        frame(cx, window, &view),
        Frame::default(),
        "re-renders never restart a finished pulse"
    );
    assert_eq!(bounds(cx, window, "url").left(), px(16.));
}
