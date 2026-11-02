use super::*;

#[gpui_kit::test]
fn volume_has_a_permanent_home_outside_the_scrolling_collection(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let mut bottom = None;
        for initial in [
            state(Phase::Ready),
            with_lists(Phase::Ready, 3, 3),
            playing(Phase::Playing, Some("Music"), 10),
            playing(Phase::Paused, Some("Music"), 10),
            state(Phase::Resolving),
        ] {
            let ((window, _, _, _), _) = open_at(cx, initial, width, height);
            let footer = bounds(cx, window, "audio-footer");
            let collection = bounds(cx, window, "queue-region");
            let volume = bounds(cx, window, "volume-percent");
            assert!(collection.bottom() <= footer.top());
            assert!(volume.top() >= footer.top() && volume.bottom() <= footer.bottom());
            assert_eq!(
                *bottom.get_or_insert_with(|| volume.bottom()),
                volume.bottom()
            );
            assert!(footer.bottom() <= px(height - 16.));
        }
    }
}

#[gpui_kit::test]
fn empty_submission_feedback_retires_on_every_edit(cx: &mut TestAppContext) {
    let (window, view, mut received, _) = open(cx, state(Phase::Ready));
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(cx.update(|cx| view.read(cx).local_hint.is_some()));
    act(cx, window, |window, cx| {
        window.input(" ", cx);
        window.press("backspace", cx);
    });
    assert!(cx.update(|cx| view.read(cx).local_hint.is_none()));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn history_actions_are_explicit_and_secondary_operations_are_contextual(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, with_lists(Phase::Ready, 2, 3));
    act(cx, window, |window, cx| window.press("ctrl-2", cx));
    assert!(present(cx, window, ("history-play", 0_usize)));
    assert!(
        button(cx, window, ("history-more", 0_usize))
            .0
            .contains("Played 0"),
        "focused action names its track"
    );
    assert!(!present(cx, window, ("history-remove", 0_usize)));
    assert!(!present(cx, window, ("history-queue", 0_usize)));
    click(cx, window, ("history-more", 0_usize));
    assert!(present(cx, window, ("history-queue", 0_usize)));
    click(cx, window, ("history-queue", 0_usize));
    assert!(
        matches!(received.try_recv(), Ok(Command::Enqueue(url)) if url == history_entry(0).url)
    );
    assert!(!present(cx, window, ("history-remove", 0_usize)));
    assert!(
        received.try_recv().is_err(),
        "opening a row/menu never plays"
    );
}

#[gpui_kit::test]
fn native_modality_changes_emphasis_without_clearing_real_focus(cx: &mut TestAppContext) {
    use gpui_kit::component::FocusableExt;
    let (window, view, _, _) = open(cx, state(Phase::Ready));
    let keyboard_ring = |cx: &mut TestAppContext| {
        act(cx, window, TestWindowExt::render_frame);
        cx.update(|cx| view.read(cx).button("probe").is_focus_ring_enabled())
    };
    act(cx, window, |window, cx| window.click("url", cx));
    assert!(!keyboard_ring(cx));
    act(cx, window, |window, cx| window.press("tab", cx));
    assert!(super::animation_fixes::focused(cx, window, "play"));
    assert!(keyboard_ring(cx));
    act(cx, window, |window, cx| window.hover("play", cx));
    assert!(
        super::animation_fixes::focused(cx, window, "play"),
        "pointer modality does not blur"
    );
    assert!(!keyboard_ring(cx));
    act(cx, window, |window, cx| window.press("shift-tab", cx));
    assert!(keyboard_ring(cx));
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input("x", cx);
    });
    assert!(super::animation_fixes::focused(cx, window, "url-editor"));
    assert!(
        !super::animation_fixes::focused(cx, window, "play"),
        "text entry cannot style a different control"
    );
}

#[gpui_kit::test]
fn row_menu_keyboard_dismissal_restores_visible_focus(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, with_lists(Phase::Ready, 3, 3));
    for _ in 0..12 {
        if super::animation_fixes::focused(cx, window, ("queue-more", 11_u64)) {
            break;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
    }
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("queue-more", 11_u64)
    ));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("queue-up", 11_u64)
    ));
    act(cx, window, |window, cx| window.press("down", cx));
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("queue-down", 11_u64)
    ));
    act(cx, window, |window, cx| window.press("escape", cx));
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("queue-more", 11_u64)
    ));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn forgetting_last_history_row_restores_the_neighbour_then_input(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 0, 2);
    let (window, view, mut received, states) = open(cx, initial.clone());
    show(cx, &view, Section::History);
    click(cx, window, ("history-remove", 1_usize));
    assert!(matches!(received.try_recv(), Ok(Command::ForgetHistory(_))));
    let mut after = initial;
    after.history.pop();
    states.send_replace(after.clone());
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("history-more", 0_usize)
    ));
    click(cx, window, ("history-remove", 0_usize));
    assert!(matches!(received.try_recv(), Ok(Command::ForgetHistory(_))));
    after.history.clear();
    states.send_replace(after);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(super::animation_fixes::focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn real_playback_with_import_and_error_keeps_two_complete_rows_above_footer(
    cx: &mut TestAppContext,
) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let mut initial = playing(Phase::Paused, Some("Żółć · 音楽 · a long track title"), 84);
        initial.queue = with_lists(Phase::Paused, 3, 3).queue;
        initial.history = with_lists(Phase::Paused, 3, 3).history;
        initial.error =
            Some("The previous seek failed. Playback remains paused. Try another position.".into());
        initial.import = crate::player::ImportStatus::Importing;
        let ((window, view, _, _), _) = open_at(cx, initial, width, height);
        for section in [Section::Queue, Section::History] {
            show(cx, &view, section);
            let list = bounds(
                cx,
                window,
                if section == Section::Queue {
                    "queue-list"
                } else {
                    "history-list"
                },
            );
            let second = bounds(
                cx,
                window,
                if section == Section::Queue {
                    ElementId::from(("queue-row", 11_u64))
                } else {
                    ElementId::from(("history-row", 1_usize))
                },
            );
            assert!(
                second.bottom() <= list.bottom(),
                "{width} {section:?}: second row must fit inside the actual viewport"
            );
            assert!(list.bottom() <= bounds(cx, window, "audio-footer").top());
        }
    }
}

#[gpui_kit::test]
fn idle_play_queue_and_section_switches_do_not_strand_keyboard_focus(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, with_lists(Phase::Ready, 3, 3));
    for _ in 0..20 {
        if super::animation_fixes::focused(cx, window, "next") {
            break;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
        });
    }
    assert!(super::animation_fixes::focused(cx, window, "next"));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Next)));
    assert!(super::animation_fixes::focused(cx, window, "url-editor"));
    let (window, _, _, _) = open(cx, with_lists(Phase::Ready, 3, 3));
    for _ in 0..20 {
        if super::animation_fixes::focused(cx, window, ("queue-more", 10_u64)) {
            break;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
        });
    }
    act(cx, window, |window, cx| window.press("ctrl-2", cx));
    assert!(super::animation_fixes::focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn clearing_history_and_cancelling_confirmation_keep_a_usable_focus_target(
    cx: &mut TestAppContext,
) {
    let initial = with_lists(Phase::Ready, 0, 2);
    let (window, view, mut received, states) = open(cx, initial.clone());
    show(cx, &view, Section::History);
    for _ in 0..20 {
        if super::animation_fixes::focused(cx, window, "history-clear") {
            break;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
        });
    }
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.press("tab", cx);
    });
    assert!(super::animation_fixes::focused(
        cx,
        window,
        "history-clear-cancel"
    ));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.simulate_next_frame(cx);
    });
    assert!(super::animation_fixes::focused(cx, window, "history-clear"));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.press("enter", cx);
    });
    assert!(matches!(received.try_recv(), Ok(Command::ClearHistory)));
    let mut after = initial;
    after.history.clear();
    states.send_replace(after);
    cx.run_until_parked();
    assert!(super::animation_fixes::focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn controller_removing_an_open_menu_target_retires_menu_and_restores_focus(
    cx: &mut TestAppContext,
) {
    for section in [Section::Queue, Section::History] {
        let initial = with_lists(Phase::Ready, 3, 3);
        let (window, view, _, states) = open(cx, initial.clone());
        show(cx, &view, section);
        click(
            cx,
            window,
            if section == Section::Queue {
                ElementId::from(("queue-more", 10_u64))
            } else {
                ElementId::from(("history-more", 0_usize))
            },
        );
        assert!(present(cx, window, "track-options-menu"));
        // An interrupted operation can restore input focus before the snapshot removes a row.
        act(cx, window, |window, cx| {
            view.update(cx, |app, cx| {
                window.focus(&app.input.read(cx).focus_handle(cx), cx);
            });
        });
        let mut after = initial;
        if section == Section::Queue {
            after.queue.remove(0);
        } else {
            after.history.remove(0);
        }
        states.send_replace(after);
        cx.run_until_parked();
        act(cx, window, |window, cx| {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
        assert!(!present(cx, window, "track-options-menu"));
        assert!(cx.update(|cx| view.read(cx).row_menu.is_none()));
        assert!(super::animation_fixes::focused(
            cx,
            window,
            if section == Section::Queue {
                ElementId::from(("queue-more", 11_u64))
            } else {
                ElementId::from(("history-more", 0_usize))
            }
        ));
    }
}

#[gpui_kit::test]
fn history_insertion_closes_old_menu_and_follows_the_same_track(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 0, 3);
    let (window, view, _, states) = open(cx, initial.clone());
    show(cx, &view, Section::History);
    click(cx, window, ("history-more", 1_usize));
    let mut after = initial;
    after.history.insert(0, history_entry(9));
    states.send_replace(after);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(!present(cx, window, "track-options-menu"));
    assert!(super::animation_fixes::focused(
        cx,
        window,
        ("history-more", 2_usize)
    ));
}

#[gpui_kit::test]
fn switching_sections_from_either_open_menu_returns_to_the_input(cx: &mut TestAppContext) {
    for trigger in [
        ElementId::from("queue-options"),
        ElementId::from(("queue-more", 10_u64)),
    ] {
        let (window, view, _, _) = open(cx, with_lists(Phase::Ready, 3, 3));
        click(cx, window, trigger);
        act(cx, window, |window, cx| window.press("ctrl-2", cx));
        assert!(super::animation_fixes::focused(cx, window, "url-editor"));
        assert!(cx.update(|cx| view.read(cx).section == Section::History));
        assert!(!present(cx, window, "track-options-menu"));
        assert!(!present(cx, window, "queue-options-menu"));
    }
}
