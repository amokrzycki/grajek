use super::support::{focused, results, settle, snapshot, tab_until};
use super::*;
use crate::search::SearchAction;
use crate::search::SearchPhase;

fn edit(cx: &mut TestAppContext, window: AnyWindowHandle, value: &str) {
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.press("ctrl-a", cx);
        window.input(value, cx);
    });
}

fn submit(cx: &mut TestAppContext, window: AnyWindowHandle, received: &mut TestCommands) {
    edit(cx, window, "Bach  cello");
    assert!(received.try_recv().is_err());
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(received.try_recv().is_err());
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(
        matches!(received.try_recv(), Ok(Command::Search { id: 1, query }) if query == "Bach cello")
    );
}

#[gpui_kit::test]
fn explicit_submission_selection_and_acknowledged_enqueue(cx: &mut TestAppContext) {
    let (window, _view, mut received, states) = open(cx, state(Phase::Ready));
    submit(cx, window, &mut received);
    assert!(present(cx, window, "search-cancel"));
    assert!(!present(cx, window, "search-row-0"));
    states.send_replace(snapshot(Phase::Ready, results()));
    settle(cx, window);
    assert!(present(cx, window, "search-row-0"));
    click(cx, window, "search-play");
    click(cx, window, "search-enqueue");
    assert!(received.try_recv().is_err());
    click(cx, window, "search-row-0");
    assert!(received.try_recv().is_err());
    click(cx, window, "search-enqueue");
    click(cx, window, "search-enqueue");
    assert!(
        matches!(received.try_recv(), Ok(Command::SearchAction { search_id: 1, video_id, action: SearchAction::Enqueue { id: 1, placement: crate::player::Placement::End } }) if video_id == "aaaaaaaaaa0")
    );
    assert!(received.try_recv().is_err());
    assert!(
        present(cx, window, "search-row-0"),
        "keep rows until controller accepts"
    );
    let mut accepted = snapshot(Phase::Ready, SearchPhase::Idle);
    accepted.queue.push(QueueItem {
        id: 42,
        url: "https://www.youtube.com/watch?v=aaaaaaaaaa0".into(),
        title: None,
        title_unavailable: false,
    });
    accepted.enqueue_result = Some(crate::player::EnqueueResult {
        request_id: 1,
        item_id: Some(42),
    });
    states.send_replace(accepted);
    settle(cx, window);
    assert!(!present(cx, window, "search-cancel"));
    assert_eq!(
        status(cx, window),
        "Added to queue · #1 · Loading title…",
        "the accepted result is confirmed in the composer"
    );
}

#[gpui_kit::test]
fn edit_cancel_and_stale_snapshots_never_restore_results(cx: &mut TestAppContext) {
    let (window, _view, mut received, states) = open(cx, state(Phase::Playing));
    submit(cx, window, &mut received);
    states.send_replace(snapshot(Phase::Playing, results()));
    settle(cx, window);
    edit(cx, window, "New query");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::CancelSearch { id: 1 })
    ));
    edit(cx, window, "New query continued");
    assert!(received.try_recv().is_err());
    states.send_replace(snapshot(Phase::Playing, results()));
    settle(cx, window);
    assert!(!present(cx, window, "search-row-0"));
    assert_eq!(input(cx, window), "New query continued");
}

#[gpui_kit::test]
fn geometry_and_keyboard_selection_stay_inside_the_clipped_list(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, _, mut received, states), _) = open_at(
            cx,
            playing(Phase::Paused, Some("Playing music"), 40),
            width,
            height,
        );
        submit(cx, window, &mut received);
        let mut next = snapshot(Phase::Paused, results());
        next.track = playing(Phase::Paused, Some("Playing music"), 40).track;
        next.error = Some("Playback recovery message wraps at the minimum window size.".into());
        next.import = crate::playlist::ImportStatus::Importing;
        states.send_replace(next);
        settle(cx, window);
        let list = bounds(cx, window, "search-list");
        let first = bounds(cx, window, "search-row-0");
        assert!(list.size.height >= px(44.), "{width}: {list:?}");
        assert!(first.top() >= list.top() && first.bottom() <= list.bottom());
        assert!(
            bounds(cx, window, "search-actions").bottom()
                <= bounds(cx, window, "audio-footer").top()
        );
        tab_until(cx, window, "search-row-0");
        act(cx, window, |window, cx| {
            window.press("end", cx);
            window.press("enter", cx);
        });
        assert!(received.try_recv().is_err());
        settle(cx, window);
        let final_row = bounds(cx, window, "search-row-4");
        assert!(
            final_row.top() >= list.top() && final_row.bottom() <= list.bottom(),
            "{final_row:?}, {list:?}"
        );
        act(cx, window, |window, cx| window.press("f1", cx));
        assert!(present(cx, window, "help-popup"));
        act(cx, window, |window, cx| window.press("escape", cx));
        assert!(
            present(cx, window, "search-row-4"),
            "Escape dismisses help first"
        );
        act(cx, window, |window, cx| window.press("escape", cx));
        assert!(matches!(
            received.try_recv(),
            Ok(Command::CancelSearch { id: 1 })
        ));
        assert!(!present(cx, window, "search-row-4"));
    }
}

#[gpui_kit::test]
fn action_refusal_retains_selection_and_retry_uses_a_new_acknowledgement(cx: &mut TestAppContext) {
    let (window, _, mut received, states) = open(cx, state(Phase::Ready));
    submit(cx, window, &mut received);
    states.send_replace(snapshot(Phase::Ready, results()));
    settle(cx, window);
    click(cx, window, "search-row-1");
    click(cx, window, "search-enqueue");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SearchAction {
            action: SearchAction::Enqueue {
                id: 1,
                placement: crate::player::Placement::End
            },
            ..
        })
    ));
    let mut rejected = snapshot(Phase::Ready, results());
    rejected.search.status = Some(crate::queue::full_message("Remove an entry and try again."));
    rejected.enqueue_result = Some(crate::player::EnqueueResult {
        request_id: 1,
        item_id: None,
    });
    states.send_replace(rejected);
    settle(cx, window);
    assert!(present(cx, window, "search-action-status"));
    assert!(!button(cx, window, "search-enqueue").1);
    click(cx, window, "search-enqueue");
    assert!(
        matches!(received.try_recv(), Ok(Command::SearchAction { search_id: 1, video_id, action: SearchAction::Enqueue { id: 2, placement: crate::player::Placement::End } }) if video_id == "aaaaaaaaaa1")
    );
}

#[gpui_kit::test]
fn search_works_without_discord_and_navigation_cancels_without_rerunning(cx: &mut TestAppContext) {
    for destination in ["settings", "search-cancel"] {
        let mut initial = state(Phase::Ready);
        initial.available = false;
        let (window, _, mut received, states) = open(cx, initial);
        submit(cx, window, &mut received);
        let mut next = snapshot(Phase::Ready, results());
        next.available = false;
        states.send_replace(next);
        settle(cx, window);
        click(cx, window, "search-row-0");
        assert!(button(cx, window, "search-play").1 && button(cx, window, "search-enqueue").1);
        click(cx, window, destination);
        assert!(matches!(
            received.try_recv(),
            Ok(Command::CancelSearch { id: 1 })
        ));
        assert!(received.try_recv().is_err());
        assert!(!present(cx, window, "search-region"));
    }
}

#[gpui_kit::test]
fn invalid_queries_and_saturated_channels_do_not_claim_submission(cx: &mut TestAppContext) {
    let (window, view, mut received, _) = open(cx, state(Phase::Ready));
    edit(cx, window, &"字".repeat(201));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(received.try_recv().is_err());
    assert!(!present(cx, window, "search-region"));
    edit(cx, window, "Music");
    cx.update(|cx| {
        let app = view.read(cx);
        for _ in 0..16 {
            assert!(app.send(Command::CancelImport));
        }
    });
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(!present(cx, window, "search-region"));
    for _ in 0..16 {
        assert!(matches!(received.try_recv(), Ok(Command::CancelImport)));
    }
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Search { id: 1, .. })
    ));
    assert!(present(cx, window, "search-region"));
}

#[gpui_kit::test]
fn errors_missing_metadata_and_section_exits_preserve_input_and_focus(cx: &mut TestAppContext) {
    let (window, _view, mut received, states) = open(cx, state(Phase::Ready));
    submit(cx, window, &mut received);
    for phase in [
        SearchPhase::Empty { rejected: 0 },
        SearchPhase::Empty { rejected: 3 },
        SearchPhase::Failed(crate::search::Error::MissingComponents),
    ] {
        states.send_replace(snapshot(Phase::Ready, phase));
        settle(cx, window);
        assert!(focused(cx, window, "url-editor"));
    }
    assert!(present(cx, window, "search-components"));
    assert!(
        !present(cx, window, "components-later"),
        "missing tools never navigate automatically"
    );
    act(cx, window, |window, cx| window.press("ctrl-2", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::CancelSearch { id: 1 })
    ));
    assert!(present(cx, window, "history-empty"));
    assert!(!present(cx, window, "queue-empty"));
    assert_eq!(input(cx, window), "Bach  cello");
    assert!(focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn selection_marks_the_row_with_an_inside_border_that_keeps_every_row_box(cx: &mut TestAppContext) {
    let (window, _, mut received, states) = open(cx, state(Phase::Ready));
    submit(cx, window, &mut received);
    states.send_replace(snapshot(Phase::Ready, results()));
    settle(cx, window);
    let row = bounds(cx, window, "search-row-0");
    let neighbour = bounds(cx, window, "search-row-1");
    let title = bounds(cx, window, "search-title-0");
    let neighbour_title = bounds(cx, window, "search-title-1");
    assert_eq!(row.size.height, px(44.));
    assert_eq!(
        title.left(),
        neighbour_title.left(),
        "every row shares one content inset before selection"
    );
    click(cx, window, "search-row-0");
    settle(cx, window);
    let selected_row = bounds(cx, window, "search-row-0");
    let selected_title = bounds(cx, window, "search-title-0");
    assert_eq!(selected_row.origin, row.origin);
    assert_eq!(selected_row.size, row.size, "the border stays inside 44px");
    assert_eq!(
        bounds(cx, window, "search-row-1"),
        neighbour,
        "an unselected row does not shift"
    );
    assert_eq!(
        selected_title.origin, title.origin,
        "selection does not shift the content"
    );
    assert_eq!(
        selected_title.left(),
        bounds(cx, window, "search-title-1").left(),
        "the selected and unselected row share one content inset"
    );
}

#[gpui_kit::test]
fn selected_action_channel_saturation_remains_retryable(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, state(Phase::Ready));
    submit(cx, window, &mut received);
    states.send_replace(snapshot(Phase::Ready, results()));
    settle(cx, window);
    click(cx, window, "search-row-0");
    let mut importing = snapshot(Phase::Ready, results());
    importing.import = crate::playlist::ImportStatus::Importing;
    states.send_replace(importing);
    settle(cx, window);
    cx.update(|cx| {
        for _ in 0..16 {
            assert!(view.read(cx).send(Command::CancelImport));
        }
    });
    click(cx, window, "search-play");
    assert!(
        present(cx, window, "search-action-status"),
        "busy search action remains visible during import"
    );
    for _ in 0..16 {
        assert!(matches!(received.try_recv(), Ok(Command::CancelImport)));
    }
    click(cx, window, "search-play");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SearchAction {
            action: SearchAction::Play,
            search_id: 1,
            ..
        })
    ));
    assert!(received.try_recv().is_err());
    assert!(present(cx, window, "search-region"));
    states.send_replace(snapshot(Phase::Connecting, SearchPhase::Idle));
    settle(cx, window);
    assert!(!present(cx, window, "search-region"));
    assert!(focused(cx, window, "url-editor"));
}
