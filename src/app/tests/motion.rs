use super::*;
use crate::app::motion::Frame;
use crate::player::{EnqueueResult, FavoriteResult, Placement};
use crate::store::FavoriteEntry;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{PlatformInput, ScrollWheelEvent};

const CANONICAL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";

fn keys(cx: &TestAppContext, view: &Entity<PlayerApp>) -> (Option<u64>, Option<u64>) {
    cx.update(|cx| view.read(cx).motion.keys())
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<PlayerApp>) -> Frame {
    act(cx, window, |window, cx| {
        window.render_frame(cx);
    });
    cx.update(|cx| view.read(cx).frame.clone())
}

fn settle_state(cx: &mut TestAppContext, window: AnyWindowHandle, next: State, states: &Snapshots) {
    states.send_replace(next);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.render_frame(cx);
    });
}

fn track(phase: Phase, id: u64, seconds: u64) -> State {
    State {
        track_id: id,
        ..playing(phase, Some("Song"), seconds)
    }
}

fn colours(cx: &TestAppContext) -> (gpui_kit::Hsla, gpui_kit::Hsla) {
    cx.update(|cx| {
        let theme = cx.theme();
        (theme.muted_foreground, theme.foreground)
    })
}

fn entry(id: u64, url: &str, title: &str) -> QueueItem {
    QueueItem {
        id,
        url: url.into(),
        title: Some(title.into()),
        title_unavailable: false,
    }
}

fn answered(mut state: State, request_id: u64, item_id: Option<u64>) -> State {
    state.enqueue_result = Some(EnqueueResult {
        request_id,
        item_id,
    });
    state
}

fn wheel(cx: &mut TestAppContext, window: AnyWindowHandle, delta: f32) {
    let center = bounds(cx, window, "queue-list").center();
    act(cx, window, |window, cx| {
        window.simulate_mouse_move(center, cx);
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: center,
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
    });
}

#[gpui_kit::test]
fn arrival_follows_the_confirmed_track_and_ends(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, state(Phase::Ready));
    assert_eq!(keys(cx, &view), (None, None), "a quiet player has no pulse");
    settle_state(
        cx,
        window,
        State {
            phase: Phase::Resolving,
            track_id: 1,
            ..state(Phase::Resolving)
        },
        &states,
    );
    assert_eq!(keys(cx, &view), (None, None), "preparing is not arrival");
    settle_state(cx, window, track(Phase::Playing, 1, 0), &states);
    assert_eq!(keys(cx, &view).0, Some(1));
    let (muted, rest) = colours(cx);
    let first = frame(cx, window, &view);
    assert_eq!(
        first.settle(1, muted, rest),
        muted,
        "starts at the transition colour"
    );
    assert_eq!(
        first.settle(2, muted, rest),
        rest,
        "another track is never tinted"
    );
    cx.executor().advance_clock(Duration::from_millis(100));
    let middle = frame(cx, window, &view).settle(1, muted, rest);
    assert!(middle != muted && middle != rest, "mid-pulse is a blend");
    cx.executor().advance_clock(Duration::from_millis(300));
    assert_eq!(
        frame(cx, window, &view),
        Frame::default(),
        "finished means resting"
    );
    assert_eq!(keys(cx, &view), (None, None), "finished pulses are dropped");
    settle_state(cx, window, track(Phase::Playing, 1, 5), &states);
    settle_state(cx, window, track(Phase::Paused, 1, 5), &states);
    settle_state(cx, window, track(Phase::Playing, 1, 5), &states);
    assert_eq!(
        keys(cx, &view),
        (None, None),
        "a tick or Pause/Resume of the same track never replays"
    );
}

#[gpui_kit::test]
fn a_new_identity_replaces_and_stop_discards_the_pulse(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, track(Phase::Playing, 1, 0));
    assert_eq!(
        keys(cx, &view),
        (None, None),
        "an already playing window does not pulse"
    );
    settle_state(
        cx,
        window,
        State {
            volume: 55,
            ..track(Phase::Playing, 1, 5)
        },
        &states,
    );
    assert_eq!(keys(cx, &view).0, None, "volume has no pulse");
    settle_state(cx, window, track(Phase::Playing, 2, 0), &states);
    assert_eq!(keys(cx, &view).0, Some(2));
    settle_state(cx, window, track(Phase::Playing, 3, 0), &states);
    assert_eq!(
        keys(cx, &view).0,
        Some(3),
        "one slot: the newest replaces the running pulse"
    );
    settle_state(cx, window, state(Phase::Ready), &states);
    assert_eq!(keys(cx, &view), (None, None), "Stop or cancel discards it");
    assert_eq!(frame(cx, window, &view), Frame::default());
}

#[gpui_kit::test]
fn settings_round_trip_never_replays_a_finished_pulse(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, state(Phase::Ready));
    settle_state(cx, window, track(Phase::Playing, 1, 0), &states);
    assert_eq!(keys(cx, &view).0, Some(1));
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(frame(cx, window, &view), Frame::default());
    for open in [true, false] {
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.settings_open = open;
                cx.notify();
            });
        });
        act(cx, window, |window, cx| {
            window.render_frame(cx);
        });
    }
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(frame(cx, window, &view), Frame::default());
    assert_eq!(keys(cx, &view), (None, None));
}

#[gpui_kit::test]
fn reduced_motion_skips_every_pulse(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, state(Phase::Ready));
    cx.update(|cx| cx.set_reduce_motion(true));
    settle_state(cx, window, track(Phase::Playing, 1, 0), &states);
    assert_eq!(frame(cx, window, &view), Frame::default());
    assert_eq!(
        keys(cx, &view),
        (None, None),
        "nothing is left to request frames"
    );
}

#[gpui_kit::test]
fn a_queue_move_is_marked_only_when_the_controller_confirms_it(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 3, 0));
    super::composition::tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, up: false })
    ));
    assert_eq!(keys(cx, &view).1, None, "asking is not a confirmation");
    settle_state(
        cx,
        window,
        State {
            position: Duration::from_secs(1),
            ..with_lists(Phase::Ready, 3, 0)
        },
        &states,
    );
    assert_eq!(
        keys(cx, &view).1,
        None,
        "an unrelated tick neither marks nor forgets the request"
    );
    let mut moved = with_lists(Phase::Ready, 3, 0);
    moved.queue.swap(0, 1);
    settle_state(cx, window, moved, &states);
    assert_eq!(keys(cx, &view).1, Some(10));
    let brass = cx.update(|cx| cx.theme().primary);
    assert!(frame(cx, window, &view).wash(10, brass).is_some());
    cx.executor().advance_clock(Duration::from_millis(400));
    assert_eq!(frame(cx, window, &view), Frame::default());
}

#[gpui_kit::test]
fn an_unanswered_move_request_expires_instead_of_marking_a_later_rotation(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 3, 0));
    super::composition::tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(received.try_recv().is_ok());
    cx.executor().advance_clock(Duration::from_secs(3));
    settle_state(cx, window, with_lists(Phase::Ready, 3, 0), &states);
    let mut rotated = with_lists(Phase::Ready, 3, 0);
    rotated.queue.rotate_left(1);
    settle_state(cx, window, rotated, &states);
    assert_eq!(
        keys(cx, &view).1,
        None,
        "a later rotation is not the user's move"
    );
}

#[gpui_kit::test]
fn an_expired_move_request_is_dropped_before_any_later_reordering(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 3, 0));
    super::composition::tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(received.try_recv().is_ok());
    cx.executor().advance_clock(Duration::from_secs(3));
    let mut rotated = with_lists(Phase::Ready, 3, 0);
    rotated.queue.rotate_left(1);
    settle_state(cx, window, rotated, &states);
    assert_eq!(
        keys(cx, &view).1,
        None,
        "the first snapshot after expiry is dropped, not marked"
    );
    act(cx, window, |window, cx| window.press("alt-up", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, up: true })
    ));
    let mut moved = with_lists(Phase::Ready, 3, 0);
    moved.queue.swap(1, 2);
    settle_state(cx, window, moved, &states);
    assert_eq!(
        keys(cx, &view).1,
        Some(10),
        "a request answered within the window still marks"
    );
}

#[gpui_kit::test]
fn a_confirmed_add_is_marked_and_removal_discards_the_mark(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input(CANONICAL, cx);
        window.press("ctrl-enter", cx);
    });
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest { id: 1, .. })
    ));
    assert_eq!(
        keys(cx, &view).1,
        None,
        "nothing before the controller answers"
    );
    let mut acknowledged = initial;
    acknowledged.queue.push(entry(99, CANONICAL, "Aqua"));
    settle_state(
        cx,
        window,
        answered(acknowledged.clone(), 1, Some(99)),
        &states,
    );
    assert_eq!(keys(cx, &view).1, Some(99));
    acknowledged.queue.pop();
    settle_state(cx, window, acknowledged, &states);
    assert_eq!(
        keys(cx, &view).1,
        None,
        "a removed entry cannot stay marked"
    );
}

#[gpui_kit::test]
fn queue_next_move_to_next_and_a_favorite_insert_are_marked_on_acknowledgement(
    cx: &mut TestAppContext,
) {
    let initial = with_lists(Phase::Ready, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input("https://youtu.be/aqz-KE-bpKQ", cx);
    });
    click(cx, window, "enqueue-next");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest {
            id: 1,
            placement: Placement::Next,
            ..
        })
    ));
    let mut acknowledged = initial;
    acknowledged.queue.insert(0, entry(99, CANONICAL, "Aqua"));
    settle_state(cx, window, answered(acknowledged, 1, Some(99)), &states);
    assert_eq!(keys(cx, &view).1, Some(99));

    click(cx, window, ("queue-next", 11_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 2,
            id: 11
        })
    ));
    let mut moved = with_lists(Phase::Ready, 2, 0);
    let shifted = moved.queue.remove(1);
    moved.queue.insert(0, shifted);
    settle_state(cx, window, answered(moved, 2, Some(11)), &states);
    assert_eq!(keys(cx, &view).1, Some(11));

    let mut with_favorite = state(Phase::Ready);
    with_favorite.favorites = vec![FavoriteEntry {
        url: CANONICAL.into(),
        title: Some("Aqua".into()),
        channel: None,
        duration_secs: None,
        live: false,
    }];
    settle_state(cx, window, with_favorite.clone(), &states);
    show(cx, &view, Section::Favorites);
    click(cx, window, format!("favorite-next:{CANONICAL}"));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest { id: 3, value, placement: Placement::Next }) if value == CANONICAL
    ));
    let mut inserted = with_favorite;
    inserted.queue.insert(0, entry(77, CANONICAL, "Aqua"));
    settle_state(cx, window, answered(inserted, 3, Some(77)), &states);
    assert_eq!(keys(cx, &view).1, Some(77));
}

#[gpui_kit::test]
fn the_mark_does_not_refire_on_unrelated_snapshots(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input(CANONICAL, cx);
        window.press("ctrl-enter", cx);
    });
    assert!(received.try_recv().is_ok());
    let mut acknowledged = initial;
    acknowledged.queue.push(entry(99, CANONICAL, "Aqua"));
    settle_state(
        cx,
        window,
        answered(acknowledged.clone(), 1, Some(99)),
        &states,
    );
    assert_eq!(keys(cx, &view).1, Some(99));
    cx.executor().advance_clock(Duration::from_millis(400));
    assert_eq!(frame(cx, window, &view), Frame::default());
    settle_state(
        cx,
        window,
        answered(
            State {
                volume: 30,
                ..acknowledged
            },
            1,
            Some(99),
        ),
        &states,
    );
    assert_eq!(
        keys(cx, &view).1,
        None,
        "a retained result is not a new confirmation"
    );
    assert_eq!(frame(cx, window, &view), Frame::default());
}

#[gpui_kit::test]
fn a_marked_row_scrolled_back_does_not_replay(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 40, 0));
    super::composition::tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, .. })
    ));
    let mut moved = with_lists(Phase::Ready, 40, 0);
    moved.queue.swap(0, 1);
    settle_state(cx, window, moved, &states);
    assert_eq!(keys(cx, &view).1, Some(10));
    for _ in 0..6 {
        wheel(cx, window, 200.);
    }
    for _ in 0..6 {
        wheel(cx, window, -200.);
    }
    let brass = cx.update(|cx| cx.theme().primary);
    assert_eq!(
        keys(cx, &view).1,
        Some(10),
        "the pulse survives the row leaving and re-entering the viewport"
    );
    assert!(frame(cx, window, &view).wash(10, brass).is_some());
    cx.executor().advance_clock(Duration::from_millis(400));
    assert_eq!(frame(cx, window, &view), Frame::default());
    wheel(cx, window, 200.);
    wheel(cx, window, -200.);
    assert!(
        frame(cx, window, &view).wash(10, brass).is_none(),
        "a finished pulse does not replay"
    );
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "window access failures must fail the wash and focus assertions"
)]
fn the_wash_yields_to_the_keyboard_focus_row_background(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input(CANONICAL, cx);
        window.press("ctrl-enter", cx);
    });
    assert!(received.try_recv().is_ok());
    let mut acknowledged = initial;
    acknowledged.queue.push(entry(99, CANONICAL, "Aqua"));
    settle_state(cx, window, answered(acknowledged, 1, Some(99)), &states);
    let brass = cx.update(|cx| cx.theme().primary);
    assert!(
        frame(cx, window, &view).wash(99, brass).is_some(),
        "the pulse is running while the row is examined"
    );
    let unfocused_row_wash = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            view.read(cx).row_wash(2, 99, window, cx)
        })
        .expect("test window should expose the wash")
    };
    assert!(
        unfocused_row_wash(cx).is_some(),
        "a row without keyboard focus shows the wash"
    );
    super::composition::tab_until(cx, window, ("queue-more", 99_u64));
    let focused = cx
        .update_window(window, |_, window, _| {
            window.find(("queue-more", 99_u64)).focused()
        })
        .expect("test window should expose the row action");
    assert_eq!(focused, Some(true));
    let wash = cx
        .update_window(window, |_, window, cx| {
            view.read(cx).row_wash(2, 99, window, cx)
        })
        .expect("test window should expose the wash");
    assert_eq!(
        wash, None,
        "the keyboard-focus row keeps its own background"
    );
    let keyboard = cx.update(|cx| view.read(cx).focus.keyboard);
    assert!(keyboard, "the filter saw a real keyboard focus");
}

#[gpui_kit::test]
fn frames_are_requested_only_while_a_pulse_runs(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, state(Phase::Ready));
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(
        requested(cx, window),
        0,
        "a resting window requests no frame"
    );
    settle_state(cx, window, track(Phase::Playing, 1, 0), &states);
    assert!(
        requested(cx, window) >= 1,
        "a running pulse keeps the frame loop alive"
    );
    cx.executor().advance_clock(Duration::from_millis(500));
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(
        requested(cx, window),
        0,
        "a finished pulse requests no further frame"
    );
    cx.update(|cx| cx.set_reduce_motion(true));
    settle_state(cx, window, track(Phase::Playing, 2, 0), &states);
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(requested(cx, window), 0, "reduced motion requests no frame");
    assert_eq!(frame(cx, window, &view), Frame::default());
}

#[gpui_kit::test]
fn error_and_notice_fades_start_only_on_a_new_message(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, state(Phase::Ready));
    let failure = State {
        error: Some("Connection lost.".into()),
        ..state(Phase::Error)
    };
    settle_state(cx, window, failure.clone(), &states);
    assert_eq!(frame(cx, window, &view).error_fade(), Some(0.0));
    cx.executor().advance_clock(Duration::from_millis(100));
    settle_state(
        cx,
        window,
        State {
            volume: 30,
            ..failure.clone()
        },
        &states,
    );
    assert!(
        frame(cx, window, &view)
            .error_fade()
            .is_some_and(|fade| fade > 0.5),
        "an unrelated snapshot does not restart the fade"
    );
    cx.executor().advance_clock(Duration::from_millis(200));
    assert_eq!(frame(cx, window, &view).error_fade(), None);
    settle_state(
        cx,
        window,
        State {
            volume: 40,
            ..failure
        },
        &states,
    );
    assert_eq!(
        frame(cx, window, &view).error_fade(),
        None,
        "the same message never refires"
    );
    let next = State {
        error: Some("Playback failed.".into()),
        ..state(Phase::Error)
    };
    settle_state(cx, window, next, &states);
    assert_eq!(frame(cx, window, &view).error_fade(), Some(0.0));

    let notice = State {
        notice: Some("Recovered history file.".into()),
        ..state(Phase::Ready)
    };
    settle_state(cx, window, notice.clone(), &states);
    assert_eq!(frame(cx, window, &view).notice_fade(), Some(0.0));
    cx.executor().advance_clock(Duration::from_millis(200));
    assert_eq!(frame(cx, window, &view).notice_fade(), None);
    settle_state(
        cx,
        window,
        State {
            volume: 50,
            ..notice
        },
        &states,
    );
    assert_eq!(
        frame(cx, window, &view).notice_fade(),
        None,
        "a retained notice never refires"
    );
}

#[gpui_kit::test]
fn an_error_present_at_startup_does_not_fade_in(cx: &mut TestAppContext) {
    let startup = State {
        error: Some("Connection lost.".into()),
        notice: Some("Recovered history file.".into()),
        ..state(Phase::Error)
    };
    let (window, view, _received, states) = open(cx, startup);
    assert_eq!(frame(cx, window, &view).error_fade(), None);
    assert_eq!(
        frame(cx, window, &view).notice_fade(),
        None,
        "a notice present at startup does not fade in"
    );
    settle_state(
        cx,
        window,
        State {
            volume: 40,
            notice: Some("Recovered history file.".into()),
            ..state(Phase::Error)
        },
        &states,
    );
    assert_eq!(
        frame(cx, window, &view).notice_fade(),
        None,
        "the startup notice never refires on the first snapshot"
    );
    settle_state(
        cx,
        window,
        State {
            notice: Some("History imported.".into()),
            ..state(Phase::Error)
        },
        &states,
    );
    assert!(
        frame(cx, window, &view).notice_fade().is_some(),
        "a later different notice starts a fade"
    );
}

#[gpui_kit::test]
fn the_saved_label_settles_only_after_a_confirmed_result(cx: &mut TestAppContext) {
    let mut initial = playing(Phase::Playing, Some("Aqua"), 5);
    initial.current_url = Some(CANONICAL.into());
    let (window, view, mut received, states) = open(cx, initial.clone());
    click(cx, window, "now-playing-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 1, url, saved: true }) if url == CANONICAL
    ));
    let mut acknowledged = initial;
    acknowledged.favorites = vec![FavoriteEntry {
        url: CANONICAL.into(),
        title: Some("Aqua".into()),
        channel: None,
        duration_secs: Some(225),
        live: false,
    }];
    acknowledged.favorite_result = Some(FavoriteResult {
        request_id: 1,
        url: CANONICAL.into(),
        saved: true,
        result: Ok(()),
    });
    settle_state(cx, window, acknowledged.clone(), &states);
    let (muted, brass) = cx.update(|cx| {
        let theme = cx.theme();
        (theme.muted_foreground, theme.primary)
    });
    assert_eq!(
        frame(cx, window, &view).saved(CANONICAL, muted, brass),
        muted,
        "the confirmation starts the settle"
    );
    cx.executor().advance_clock(Duration::from_millis(100));
    let middle = frame(cx, window, &view).saved(CANONICAL, muted, brass);
    assert!(middle != muted && middle != brass, "mid-settle is a blend");
    cx.executor().advance_clock(Duration::from_millis(200));
    assert_eq!(
        frame(cx, window, &view).saved(CANONICAL, muted, brass),
        brass
    );
    settle_state(
        cx,
        window,
        State {
            volume: 30,
            ..acknowledged
        },
        &states,
    );
    assert_eq!(
        frame(cx, window, &view).saved(CANONICAL, muted, brass),
        brass,
        "a retained result is not a new confirmation"
    );
}
