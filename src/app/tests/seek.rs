use super::*;
use crate::player::SeekStatus;
use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};

fn slider(cx: &TestAppContext, view: &Entity<PlayerApp>) -> (&'static str, gpui_kit::EntityId) {
    cx.update(|cx| ("slider", view.read(cx).progress.entity_id()))
}

fn shown(cx: &TestAppContext, view: &Entity<PlayerApp>) -> f32 {
    cx.update(|cx| view.read(cx).progress.read(cx).value().end())
}

fn pointer(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    phase: &str,
) {
    act(cx, window, |window, cx| {
        match phase {
            "down" => {
                window.dispatch_event(
                    MouseDownEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: gpui_kit::Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            "up" => {
                window.dispatch_event(
                    MouseUpEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: gpui_kit::Modifiers::default(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            _ => {
                window.dispatch_event(
                    MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: gpui_kit::Modifiers::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
            }
        }
        window.render_frame(cx);
    });
}

#[gpui_kit::test]
#[expect(
    clippy::panic,
    reason = "a missing seek command must fail the unit-returning GPUI test"
)]
fn native_seek_drag_previews_without_commands_and_reconciles_actual_position(
    cx: &mut TestAppContext,
) {
    let initial = playing(Phase::Playing, Some("Song"), 45);
    let (window, view, mut received, states) = open(cx, initial.clone());
    assert!((shown(cx, &view) - 200.).abs() < 1.);
    let track = bounds(cx, window, slider(cx, &view));
    let target = point(track.left() + track.size.width * 0.7, track.center().y);
    let thumb = point(track.left() + track.size.width * 0.2, track.center().y);
    pointer(cx, window, thumb, "down");
    // The first move claims the kit's native drag; subsequent moves update its value.
    pointer(cx, window, point(thumb.x + px(12.), thumb.y), "move");
    pointer(cx, window, target, "move");
    assert!(
        received.try_recv().is_err(),
        "pointer movement must never seek"
    );
    let proposed = shown(cx, &view);
    assert!(proposed > 600.);
    assert!(present(cx, window, "seek-status"));
    assert_eq!(
        bounds(cx, window, slider(cx, &view)).size.width,
        track.size.width,
        "preview must not resize the slider during dragging"
    );
    states.send_replace(State {
        position: Duration::from_secs(46),
        ..initial.clone()
    });
    cx.run_until_parked();
    assert!(
        (shown(cx, &view) - proposed).abs() < 1.,
        "snapshot must not move thumb while dragging"
    );
    pointer(cx, window, target, "up");
    let Ok(Command::Seek { track_id, position }) = received.try_recv() else {
        panic!("release must seek");
    };
    assert_eq!(track_id, 1);
    assert!((145..=170).contains(&position.as_secs()));
    assert!(
        received.try_recv().is_err(),
        "exactly one command per release"
    );
    assert!(
        (shown(cx, &view) - 46. / 225. * 1000.).abs() < 1.,
        "release returns to confirmed position"
    );
    states.send_replace(State {
        seek: SeekStatus::Complete(Duration::from_secs(150)),
        position: Duration::from_secs(150),
        ..initial
    });
    cx.run_until_parked();
    assert!((shown(cx, &view) - 150. / 225. * 1000.).abs() < 1.);
}

#[gpui_kit::test]
fn replaced_track_invalidates_drag_and_returns_keyboard_focus(cx: &mut TestAppContext) {
    let initial = playing(Phase::Playing, Some("A"), 10);
    let (window, view, mut received, states) = open(cx, initial.clone());
    let target = bounds(cx, window, slider(cx, &view)).center();
    pointer(cx, window, target, "down");
    pointer(cx, window, target, "move");
    states.send_replace(State {
        track_id: 2,
        position: Duration::ZERO,
        ..initial
    });
    cx.run_until_parked();
    pointer(cx, window, target, "move");
    assert!(
        shown(cx, &view).abs() < 1.,
        "rejected stale movement must immediately restore the confirmed thumb"
    );
    pointer(cx, window, target, "up");
    assert!(
        received.try_recv().is_err(),
        "old gesture cannot seek replacement track"
    );
    assert!(shown(cx, &view).abs() < 1.);
    act(cx, window, |window, cx| {
        assert!(
            view.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
}

#[gpui_kit::test]
fn seeking_keyboard_is_scoped_to_native_slider_focus(cx: &mut TestAppContext) {
    let initial = playing(Phase::Paused, Some("Song"), 30);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| window.press("right", cx));
    assert!(
        received.try_recv().is_err(),
        "URL editor retains arrow keys"
    );
    act(cx, window, |window, cx| {
        let focus = view.read(cx).progress_focus.clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
    });
    for (key, seconds) in [
        ("right", 35),
        ("left", 25),
        ("pageup", 40),
        ("pagedown", 20),
        ("home", 0),
        ("end", 224),
    ] {
        act(cx, window, |window, cx| window.press(key, cx));
        assert!(
            matches!(received.try_recv(), Ok(Command::Seek { track_id: 1, position }) if position.as_secs() == seconds),
            "{key}"
        );
        assert!(
            (shown(cx, &view) - 30. / 225. * 1000.).abs() < 1.,
            "key does not invent playback position"
        );
    }
    states.send_replace(State {
        seek: SeekStatus::Failed("Cannot seek this stream".into()),
        ..initial
    });
    cx.run_until_parked();
    assert!(present(cx, window, "error"));
    act(cx, window, |window, cx| window.press("right", cx));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn progress_slider_disabled_states_and_compact_layout(cx: &mut TestAppContext) {
    for (width, height) in [(420., 480.), (520., 560.)] {
        let initial = playing(Phase::Playing, Some(&"Long title ".repeat(80)), 30);
        let ((window, view, mut received, states), _) = open_at(cx, initial.clone(), width, height);
        for seek in [
            SeekStatus::Ready,
            SeekStatus::Pending(Duration::from_secs(50)),
            SeekStatus::Failed("Seek failed".repeat(50)),
        ] {
            states.send_replace(State {
                seek,
                ..initial.clone()
            });
            cx.run_until_parked();
            let bar = bounds(cx, window, "seek-slider");
            let transport = bounds(cx, window, "stop");
            let queue = bounds(cx, window, "queue-empty");
            assert!(bar.size.width > gpui_kit::px(100.) && bar.right() < gpui_kit::px(width));
            assert!(transport.bottom() < gpui_kit::px(height));
            assert!(queue.size.height > gpui_kit::px(24.), "queue stays usable");
        }
        for phase in [
            Phase::Ready,
            Phase::Resolving,
            Phase::Stopping,
            Phase::Disconnecting,
            Phase::Error,
        ] {
            states.send_replace(state(phase));
            cx.run_until_parked();
            assert!(!present(cx, window, "seek-slider"));
        }
        let mut unknown = initial.clone();
        if let Some(track) = &mut unknown.track {
            track.duration = None;
        }
        states.send_replace(unknown);
        cx.run_until_parked();
        assert!(!present(cx, window, "seek-slider"));
        let mut offline = initial;
        offline.available = false;
        states.send_replace(offline);
        cx.run_until_parked();
        click(cx, window, slider(cx, &view));
        assert!(received.try_recv().is_err());
    }
}

#[gpui_kit::test]
fn accessible_seek_actions_use_controller_intents_and_reject_disabled_or_invalid_values(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{AccessibleAction, accesskit::ActionData};
    let initial = playing(Phase::Paused, Some("Song"), 30);
    let (window, view, mut received, states) = open(cx, initial.clone());
    let action = |cx: &mut TestAppContext, action, data: Option<ActionData>| {
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.progress_accessibility(action, data.as_ref(), cx);
            });
        });
    };
    for (kind, data, expected) in [
        (AccessibleAction::Increment, None, 35),
        (AccessibleAction::Decrement, None, 25),
        (
            AccessibleAction::SetValue,
            Some(ActionData::NumericValue(90.)),
            90,
        ),
        (
            AccessibleAction::SetValue,
            Some(ActionData::NumericValue(1e12)),
            224,
        ),
    ] {
        action(cx, kind, data);
        assert!(
            matches!(received.try_recv(), Ok(Command::Seek { position, .. }) if position.as_secs() == expected)
        );
        assert!(
            received.try_recv().is_err(),
            "one intent per accessibility action"
        );
        assert!((shown(cx, &view) - 30. / 225. * 1000.).abs() < 1.);
    }
    for seconds in [-1., f64::NAN, f64::INFINITY, 1e300] {
        action(
            cx,
            AccessibleAction::SetValue,
            Some(ActionData::NumericValue(seconds)),
        );
        assert!(received.try_recv().is_err());
    }
    act(cx, window, |window, _| {
        assert_eq!(
            window.find("seek-slider").role(),
            Some(gpui_kit::Role::Slider)
        );
    });
    states.send_replace(State {
        available: false,
        ..initial
    });
    cx.run_until_parked();
    action(cx, AccessibleAction::Increment, None);
    action(
        cx,
        AccessibleAction::SetValue,
        Some(ActionData::NumericValue(20.)),
    );
    assert!(received.try_recv().is_err());
}
