use super::*;
use gpui_kit::{IntoElement, PlatformInput, ScrollWheelEvent};
use std::{hint::black_box, time::Instant};

thread_local! {
    static ROW_COST: std::cell::Cell<(u64, u128)> = const { std::cell::Cell::new((0, 0)) };
}

pub(in crate::app) struct RowTimer(Instant);

impl RowTimer {
    pub(in crate::app) fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for RowTimer {
    fn drop(&mut self) {
        ROW_COST.with(|cost| {
            let (count, nanos) = cost.get();
            cost.set((count + 1, nanos + self.0.elapsed().as_nanos()));
        });
    }
}

fn long_lists(phase: Phase, queue: u64, history: usize) -> State {
    let mut snapshot = with_lists(phase, queue, history);
    if phase == Phase::Playing {
        snapshot.track = playing(phase, Some("Playback benchmark"), 0).track;
        snapshot.track_id = 1;
    }
    for item in &mut snapshot.queue {
        item.title = Some("Żółć long queue title with realistic metadata ".repeat(8));
    }
    for entry in &mut snapshot.history {
        entry.title = Some("Żółć long history title with realistic metadata ".repeat(8));
    }
    snapshot
}

fn wheel(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    delta: f32,
) {
    act(cx, window, |window, cx| {
        window.simulate_mouse_move(position, cx);
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
    });
}

#[gpui_kit::test]
fn play_stays_available_during_pause_and_resume(cx: &mut TestAppContext) {
    for phase in [Phase::Playing, Phase::Paused] {
        let (window, view, mut received, states) = open(cx, long_lists(phase, 5, 5));
        assert!(!button(cx, window, "play").1);
        click(cx, window, "pause-resume");
        assert!(matches!(
            received.try_recv(),
            Ok(Command::Pause | Command::Resume)
        ));
        assert!(
            !button(cx, window, "play").1,
            "Play URL must keep its normal enabled presentation"
        );
        show(cx, &view, Section::History);
        assert!(!button(cx, window, ("history-play", 0_usize)).1);
        let mut response = long_lists(phase, 5, 5);
        response.phase = if phase == Phase::Playing {
            Phase::Paused
        } else {
            Phase::Playing
        };
        states.send_replace(response);
        cx.run_until_parked();
        assert!(!button(cx, window, "play").1);
    }
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "small fixed test indices must convert or fail the test"
)]
fn removal_edges_dispatch_outside_the_scrollbar_lane(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for (width, height) in [(420., 480.), (520., 560.)] {
        for section in [Section::Queue, Section::History] {
            let ((window, view, mut received, _states), _) =
                open_at(cx, long_lists(Phase::Ready, 100, 50), width, height);
            show(cx, &view, section);
            let first = if section == Section::Queue {
                ElementId::from(("queue-row", 10_u64))
            } else {
                ElementId::from(("history-row", 0_usize))
            };
            let top = bounds(cx, window, first.clone()).top();
            let row = bounds(cx, window, first.clone());
            let viewport = Bounds::from_corners(
                row.origin,
                point(bounds(cx, window, "url").right(), px(height - 16.)),
            );
            let pointer = point(viewport.center().x, top + px(10.));
            for (delta, index) in [
                (0., 0_usize),
                (-720., 20),
                (-10000., if section == Section::Queue { 99 } else { 49 }),
            ] {
                wheel(cx, window, pointer, delta);
                let id = if section == Section::Queue {
                    ElementId::from((
                        "queue-remove",
                        10 + u64::try_from(index).expect("small index"),
                    ))
                } else {
                    ElementId::from(("history-remove", index))
                };
                let control = bounds(cx, window, id.clone());
                // A pointer move reveals the active overlay before the actual down/up events.
                act(cx, window, |window, cx| {
                    window.click_at(
                        id,
                        point(control.size.width - px(1.), control.size.height / 2.),
                        cx,
                    );
                });
                let command = received.try_recv();
                match section {
                    Section::Queue => {
                        if !matches!(command, Ok(Command::Dequeue(id)) if id == 10 + u64::try_from(index).expect("small index"))
                        {
                            failures.push(format!(
                                "{width} {section:?} {index}: edge click intercepted"
                            ));
                        }
                    }
                    Section::History => {
                        if !matches!(&command, Ok(Command::ForgetHistory(url)) if *url == history_entry(index).url)
                        {
                            failures.push(format!(
                                "{width} {section:?} {index}: edge click intercepted"
                            ));
                        }
                    }
                }
                if control.right() > viewport.right() - px(16.) {
                    failures.push(format!(
                        "{width} {section:?} {index}: removal overlaps scrollbar lane by {:?}",
                        control.right() - (viewport.right() - px(16.))
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui_kit::test]
#[ignore = "opt-in CPU benchmark; see plans/evidence.md"]
#[expect(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "benchmark emits raw samples; fixed small sample counts fit f64"
)]
fn scrolling_cpu_benchmark(cx: &mut TestAppContext) {
    const WARMUP: usize = 20;
    const SAMPLES: usize = 600;
    println!(
        "section,width,height,rows,phase,construct_median_us,construct_p95_us,frame_median_us,frame_p95_us,row_construct_median_us,row_construct_p95_us,max_rows_per_draw,update_frames_median_us,update_frames_p95_us,first_row_y,last_row_y"
    );
    for (width, height) in [(420., 480.), (520., 560.)] {
        for section in [Section::Queue, Section::History] {
            for count in [5, if section == Section::Queue { 100 } else { 50 }] {
                for phase in [Phase::Ready, Phase::Playing] {
                    let mut initial = long_lists(
                        phase,
                        if section == Section::Queue { count } else { 0 },
                        if section == Section::History {
                            usize::try_from(count).expect("small count")
                        } else {
                            0
                        },
                    );
                    let ((window, view, _received, states), _) =
                        open_at(cx, initial.clone(), width, height);
                    show(cx, &view, section);
                    let first = if section == Section::Queue {
                        ElementId::from(("queue-row", 10_u64))
                    } else {
                        ElementId::from(("history-row", 0_usize))
                    };
                    let first_bounds = bounds(cx, window, first.clone());
                    let pointer = first_bounds.center();
                    let mut construction = Vec::new();
                    let mut frames = Vec::new();
                    let mut row_costs = Vec::new();
                    let mut max_rows = 0;
                    let mut updates = Vec::new();
                    for sample in 0..WARMUP + SAMPLES {
                        let start = Instant::now();
                        cx.update(|cx| {
                            view.update(cx, |app, cx| {
                                match section {
                                    Section::Queue => {
                                        black_box(app.queue_list(cx).into_any_element())
                                    }
                                    Section::History => {
                                        black_box(app.history_list(cx).into_any_element())
                                    }
                                };
                            });
                        });
                        let construct_us = start.elapsed().as_secs_f64() * 1e6;
                        ROW_COST.with(|cost| cost.set((0, 0)));
                        let start = Instant::now();
                        let update = phase == Phase::Playing && sample % 60 == 0;
                        if update {
                            initial.position += Duration::from_secs(1);
                            states.send_replace(initial.clone());
                            cx.run_until_parked();
                        }
                        wheel(
                            cx,
                            window,
                            pointer,
                            if sample % 120 < 60 { -12. } else { 12. },
                        );
                        let frame_us = start.elapsed().as_secs_f64() * 1e6;
                        let (rows, nanos) = ROW_COST.with(std::cell::Cell::get);
                        cx.executor()
                            .advance_clock(Duration::from_nanos(16_666_667));
                        if sample >= WARMUP {
                            construction.push(construct_us);
                            frames.push(frame_us);
                            row_costs.push(nanos as f64 / 1000.);
                            max_rows = max_rows.max(if update { rows / 2 } else { rows });
                            if update {
                                updates.push(frame_us);
                            }
                        }
                    }
                    let mut observed = Vec::new();
                    act(cx, window, |window, _| {
                        for index in 0..count {
                            let id = match section {
                                Section::Queue => ElementId::from(("queue-row", 10 + index)),
                                Section::History => ElementId::from((
                                    "history-row",
                                    usize::try_from(index).expect("small count"),
                                )),
                            };
                            if let Some(row) = window.try_find(id) {
                                observed.push((index, row.bounds().top()));
                            }
                        }
                    });
                    let first_y = observed.first().map(|(_, y)| *y);
                    let last_y = observed.last().map(|(_, y)| *y);
                    let summary = |values: &mut Vec<f64>| {
                        values.sort_by(f64::total_cmp);
                        if values.is_empty() {
                            return (0., 0.);
                        }
                        (
                            values[values.len() / 2],
                            values[(values.len() * 95 / 100).min(values.len() - 1)],
                        )
                    };
                    let (cm, cp) = summary(&mut construction);
                    let (fm, fp) = summary(&mut frames);
                    let (um, up) = summary(&mut updates);
                    let (rm, rp) = summary(&mut row_costs);
                    println!(
                        "{section:?},{width},{height},{count},{phase:?},{cm:.2},{cp:.2},{fm:.2},{fp:.2},{rm:.2},{rp:.2},{max_rows},{um:.2},{up:.2},{first_y:?},{last_y:?}"
                    );
                    println!("OBSERVED {section:?}/{width}/{count}/{phase:?} {observed:?}");
                    println!(
                        "RAW {section:?}/{width}/{count}/{phase:?} construction={construction:?} frames={frames:?} rows={row_costs:?} updates={updates:?}"
                    );
                }
            }
        }
    }
}

#[gpui_kit::test]
fn offscreen_rows_do_not_participate_in_each_frame(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, long_lists(Phase::Ready, 100, 50));
    assert!(
        !present(cx, window, ("queue-row", 109_u64)),
        "offscreen queue rows are still built and laid out"
    );
    show(cx, &view, Section::History);
    assert!(
        !present(cx, window, ("history-row", 49_usize)),
        "offscreen history rows are still built and laid out"
    );
    for section in [Section::Queue, Section::History] {
        show(cx, &view, section);
        act(cx, window, gpui_kit::test::TestWindowExt::render_frame);
        ROW_COST.with(|cost| cost.set((0, 0)));
        act(cx, window, gpui_kit::test::TestWindowExt::render_frame);
        let (rows, _) = ROW_COST.with(std::cell::Cell::get);
        assert!(
            (1..=8).contains(&rows),
            "{section:?}: steady frame constructed {rows} rows"
        );
    }
}

#[gpui_kit::test]
fn independent_scroll_positions_survive_playback_and_other_snapshots(cx: &mut TestAppContext) {
    let initial = long_lists(Phase::Playing, 99, 49);
    let (window, view, _received, states) = open(cx, initial.clone());
    let first = bounds(cx, window, ("queue-row", 10_u64));
    wheel(cx, window, first.center(), -720.);
    let queue_before = bounds(cx, window, ("queue-row", 30_u64));
    show(cx, &view, Section::History);
    let first = bounds(cx, window, ("history-row", 0_usize));
    wheel(cx, window, first.center(), -360.);
    let history_before = bounds(cx, window, ("history-row", 10_usize));
    for phase in [Phase::Playing, Phase::Paused, Phase::Playing] {
        let mut snapshot = initial.clone();
        snapshot.phase = phase;
        snapshot.position = Duration::from_secs(42);
        snapshot.volume = 62;
        snapshot.queue.push(queue_item(109));
        snapshot.history.push(history_entry(49));
        states.send_replace(snapshot);
        cx.run_until_parked();
        assert_eq!(
            bounds(cx, window, ("history-row", 10_usize)).top(),
            history_before.top()
        );
        show(cx, &view, Section::Queue);
        assert_eq!(
            bounds(cx, window, ("queue-row", 30_u64)).top(),
            queue_before.top()
        );
        show(cx, &view, Section::History);
    }
}

#[gpui_kit::test]
fn pause_then_play_accepts_one_intent_and_keeps_it_protected(cx: &mut TestAppContext) {
    for enter in [false, true] {
        let initial = long_lists(Phase::Playing, 5, 5);
        let (window, view, mut received, states) = open(cx, initial.clone());
        act(cx, window, |window, cx| {
            window.click("url", cx);
            window.input("https://youtu.be/aqz-KE-bpKQ", cx);
        });
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        if enter {
            act(cx, window, |window, cx| {
                window.click("url", cx);
                window.press("enter", cx);
            });
        } else {
            click(cx, window, "play");
        }
        // A deferred intent may serialize the actual command behind Pause.
        assert!(
            button(cx, window, "play").1,
            "exactly one Play intent was accepted"
        );
        click(cx, window, "play");
        states.send_replace(State {
            phase: Phase::Paused,
            ..initial.clone()
        });
        cx.run_until_parked();
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url.contains("aqz-KE-bpKQ"))
        );
        assert!(received.try_recv().is_err());
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
        assert!(
            button(cx, window, "play").1,
            "the older Pause response cannot release Play"
        );
        click(cx, window, "play");
        assert!(received.try_recv().is_err());
        states.send_replace(State {
            phase: Phase::Paused,
            error: Some("Invalid URL".into()),
            ..initial
        });
        cx.run_until_parked();
        assert!(
            !button(cx, window, "play").1,
            "same-phase Play errors release its lock"
        );
        assert!(present(cx, window, "error"));
    }
}

fn focused(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    let mut result = false;
    act(cx, window, |window, _| {
        result = window
            .try_find(id)
            .is_some_and(|button| button.focused() == Some(true));
    });
    result
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "fixed test indices and nonempty action list are asserted by the fixture"
)]
fn every_row_action_remains_keyboard_accessible(cx: &mut TestAppContext) {
    for section in [Section::Queue, Section::History] {
        let (window, view, mut received, _states) = open(cx, long_lists(Phase::Ready, 12, 12));
        show(cx, &view, section);
        let first = if section == Section::Queue {
            ElementId::from(("queue-down", 10_u64))
        } else {
            ElementId::from(("history-play", 0_usize))
        };
        // Native focus traversal enters the list from the controls above it.
        for _ in 0..30 {
            if focused(cx, window, first.clone()) {
                break;
            }
            act(cx, window, |window, cx| {
                window.press("tab", cx);
                window.simulate_next_frame(cx);
                window.render_frame(cx);
            });
        }
        assert!(focused(cx, window, first));
        let mut order = Vec::new();
        for index in 0..12_usize {
            for action in 0..3 {
                if section == Section::Queue
                    && ((index == 0 && action == 0) || (index == 11 && action == 1))
                {
                    continue;
                }
                order.push(match section {
                    Section::Queue => ElementId::from((
                        ["queue-up", "queue-down", "queue-remove"][action],
                        10 + u64::try_from(index).expect("small index"),
                    )),
                    Section::History => ElementId::from((
                        ["history-play", "history-queue", "history-remove"][action],
                        index,
                    )),
                });
            }
        }
        for (index, id) in order.iter().enumerate() {
            assert!(
                focused(cx, window, id.clone()),
                "{section:?} {index}: {id:?}"
            );
            let rect = bounds(cx, window, id.clone());
            assert!(
                rect.top() >= px(280.) && rect.bottom() <= px(544.),
                "keyboard focus must be on screen: {index} {id:?} {rect:?}"
            );
            if index + 1 < order.len() {
                act(cx, window, |window, cx| {
                    window.press("tab", cx);
                    window.simulate_next_frame(cx);
                    window.render_frame(cx);
                });
            }
        }
        act(cx, window, |window, cx| window.press("enter", cx));
        match section {
            Section::Queue => assert!(matches!(received.try_recv(), Ok(Command::Dequeue(21)))),
            Section::History => assert!(
                matches!(received.try_recv(), Ok(Command::ForgetHistory(url)) if url == history_entry(11).url)
            ),
        }
        // Wheel scrolling a focused row away must not break the next keyboard traversal.
        let last = bounds(cx, window, order.last().expect("nonempty actions").clone());
        wheel(cx, window, last.center(), 10000.);
        for id in order.iter().rev().skip(1) {
            act(cx, window, |window, cx| {
                window.press("shift-tab", cx);
                window.simulate_next_frame(cx);
                window.render_frame(cx);
            });
            assert!(
                focused(cx, window, id.clone()),
                "reverse traversal lost {id:?}"
            );
        }
    }
}

#[gpui_kit::test]
fn unrelated_or_older_snapshots_cannot_unlock_transport(cx: &mut TestAppContext) {
    let initial = long_lists(Phase::Playing, 5, 5);
    let (window, view, mut received, states) = open(cx, initial.clone());
    click(cx, window, "pause-resume");
    assert!(matches!(received.try_recv(), Ok(Command::Pause)));
    let pause_ack = states.acknowledged.get();
    show(cx, &view, Section::History);
    click(cx, window, ("history-play", 0_usize));
    assert!(matches!(received.try_recv(), Ok(Command::Play(_))));
    let play_ack = states.acknowledged.get();
    for phase in [Phase::Playing, Phase::Paused] {
        states.sender.send_replace(State {
            phase,
            transport_ack: pause_ack,
            position: Duration::from_secs(72),
            ..initial.clone()
        });
        cx.run_until_parked();
        assert!(button(cx, window, "play").1);
        assert!(button(cx, window, ("history-play", 0_usize)).1);
        act(cx, window, |window, cx| {
            window.click("url", cx);
            window.press("enter", cx);
        });
        click(cx, window, ("history-play", 0_usize));
        assert!(received.try_recv().is_err());
    }
    states.sender.send_replace(State {
        phase: Phase::Paused,
        transport_ack: play_ack,
        error: Some("Invalid URL".into()),
        ..initial
    });
    cx.run_until_parked();
    assert!(!button(cx, window, "play").1);
    assert!(!button(cx, window, ("history-play", 0_usize)).1);
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "window access failure must fail focus regression"
)]
fn focused_play_enter_and_history_play_work_during_resume(cx: &mut TestAppContext) {
    for history in [false, true] {
        let initial = long_lists(Phase::Paused, 5, 5);
        let (window, view, mut received, states) = open(cx, initial.clone());
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Resume)));
        let id = if history {
            show(cx, &view, Section::History);
            ElementId::from(("history-play", 0_usize))
        } else {
            ElementId::from("play")
        };
        for _ in 0..30 {
            if focused(cx, window, id.clone()) {
                break;
            }
            act(cx, window, |window, cx| {
                window.press("tab", cx);
                window.simulate_next_frame(cx);
                window.render_frame(cx);
            });
        }
        assert!(focused(cx, window, id));
        act(cx, window, |window, cx| window.press("enter", cx));
        assert!(matches!(received.try_recv(), Ok(Command::Play(_))));
        assert!(received.try_recv().is_err());
        cx.update_window(window, |_, window, cx| {
            assert!(
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
        .expect("accepted Play rescues focus to the URL field");
        states.send_replace(State {
            phase: Phase::Resolving,
            ..initial.clone()
        });
        cx.run_until_parked();
        assert!(button(cx, window, "play").1);
        states.send_replace(State {
            track_id: 2,
            phase: Phase::Playing,
            ..initial
        });
        cx.run_until_parked();
        assert!(!button(cx, window, "play").1);
        cx.update(|cx| assert_eq!(view.read(cx).state.track_id, 2));
    }
}

#[gpui_kit::test]
fn full_or_closed_command_channel_does_not_claim_play(cx: &mut TestAppContext) {
    for phase in [Phase::Ready, Phase::Playing] {
        let (window, view, mut received, states) = open(cx, long_lists(phase, 5, 5));
        if phase == Phase::Playing {
            click(cx, window, "pause-resume");
            assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        }
        cx.update(|cx| {
            for _ in 0..16 {
                assert!(
                    view.read(cx)
                        .commands
                        .try_send(Command::Enqueue("queued".into()))
                        .is_ok()
                );
            }
        });
        click(cx, window, "play");
        assert!(
            !button(cx, window, "play").1,
            "failed sends keep Play retryable"
        );
        for _ in 0..16 {
            assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
        }
        assert!(received.try_recv().is_err());
        states.send_replace(long_lists(phase, 5, 5));
        cx.run_until_parked();
        drop(received);
        click(cx, window, "play");
        assert!(!button(cx, window, "play").1);
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, phase));
    }
}

#[gpui_kit::test]
fn scrollbar_lane_clicks_and_thumb_drags_do_not_remove_rows(cx: &mut TestAppContext) {
    use gpui_kit::{InputEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    for section in [Section::Queue, Section::History] {
        let (window, view, mut received, _states) = open(cx, long_lists(Phase::Ready, 100, 50));
        show(cx, &view, section);
        let id = match section {
            Section::Queue => "queue-list",
            Section::History => "history-list",
        };
        let viewport = bounds(cx, window, id);
        act(cx, window, |window, cx| {
            window.click_at(
                id,
                point(viewport.size.width - px(8.), viewport.size.height - px(12.)),
                cx,
            );
        });
        assert!(received.try_recv().is_err());
        let offset = cx.update(|cx| match section {
            Section::Queue => view.read(cx).queue_scroll.state.logical_scroll_top(),
            Section::History => view.read(cx).history_scroll.state.logical_scroll_top(),
        });
        assert!(
            offset.item_ix > 0 || offset.offset_in_item > px(0.),
            "track click must scroll"
        );
        // Reset with the same persistent native handle, then drag the thumb through its lane.
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                match section {
                    Section::Queue => &app.queue_scroll.state,
                    Section::History => &app.history_scroll.state,
                }
                .scroll_to(gpui_kit::ListOffset::default());
                cx.notify();
            });
        });
        act(cx, window, |window, cx| {
            window.render_frame(cx);
            let start = point(viewport.right() - px(8.), viewport.top() + px(5.));
            let end = point(start.x, viewport.bottom() - px(5.));
            window.simulate_mouse_move(start, cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: start,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseMoveEvent {
                    position: end,
                    pressed_button: Some(MouseButton::Left),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: end,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        });
        assert!(received.try_recv().is_err());
        let last = match section {
            Section::Queue => ElementId::from(("queue-row", 109_u64)),
            Section::History => ElementId::from(("history-row", 49_usize)),
        };
        assert!(
            present(cx, window, last),
            "thumb drag must reach the final row"
        );
    }
}
