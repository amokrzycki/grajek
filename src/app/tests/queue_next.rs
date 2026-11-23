//! Queue next and Move to next: every entry point, the acknowledgement-only feedback, and
//! the focus and layout rules at both native sizes.
use super::animation_fixes::focused;
use super::composition::tab_until;
use super::search::{results, snapshot};
use super::*;
use crate::app::composer::Landing;
use crate::player::{EnqueueResult, Placement, RepeatMode, SearchAction, SearchPhase};

const VIDEO: &str = "https://youtu.be/aqz-KE-bpKQ";
const CANONICAL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const SIZES: [(f32, f32); 2] = [(520., 560.), (420., 480.)];

fn status(cx: &mut TestAppContext, window: AnyWindowHandle) -> String {
    let mut text = String::new();
    act(cx, window, |window, _| {
        text = window
            .find("link-status")
            .label()
            .unwrap_or_default()
            .to_owned();
    });
    text
}

fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
}

fn type_link(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.press("ctrl-a", cx);
        window.input(text, cx);
    });
}

fn input(cx: &TestAppContext, view: &Entity<PlayerApp>) -> String {
    cx.update(|cx| view.read(cx).input.read(cx).value().to_string())
}

fn answered(mut state: State, request_id: u64, item_id: Option<u64>) -> State {
    state.enqueue_result = Some(EnqueueResult {
        request_id,
        item_id,
    });
    state
}

fn entry(id: u64, url: &str, title: &str) -> QueueItem {
    QueueItem {
        id,
        url: url.into(),
        title: Some(title.into()),
        title_unavailable: false,
    }
}

fn next_request(received: &mut TestCommands) -> Option<(u64, String)> {
    match received.try_recv() {
        Ok(Command::EnqueueRequest {
            id,
            value,
            placement: Placement::Next,
        }) => Some((id, value)),
        _ => None,
    }
}

#[test]
fn the_queue_next_icon_is_bundled_with_the_app() {
    use gpui_kit::AssetSource as _;
    let icon = crate::AppAssets.load("icons/list-start.svg");
    assert!(matches!(icon, Ok(Some(bytes)) if !bytes.is_empty()));
}

#[gpui_kit::test]
fn composer_queue_next_sends_one_request_and_claims_nothing_until_acknowledged(
    cx: &mut TestAppContext,
) {
    let initial = with_lists(Phase::Playing, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert_eq!(next_request(&mut received), Some((1, VIDEO.into())));
    click(cx, window, "enqueue-next");
    assert!(
        received.try_recv().is_err(),
        "an unanswered identical text is not resent"
    );
    assert_eq!(input(cx, &view), VIDEO);
    assert!(!status(cx, window).starts_with("Queued next"));
    states.send_replace(State {
        position: Duration::from_secs(1),
        ..initial.clone()
    });
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        VIDEO,
        "unrelated snapshots acknowledge nothing"
    );
    let mut acknowledged = initial;
    acknowledged.queue.insert(0, entry(99, CANONICAL, "Aqua"));
    states.send_replace(answered(acknowledged, 1, Some(99)));
    settle(cx, window);
    assert_eq!(input(cx, &view), "");
    assert_eq!(status(cx, window), "Queued next · Aqua");
    cx.update(|cx| assert_eq!(view.read(cx).added, Some((99, Landing::Next))));
}

#[gpui_kit::test]
fn queued_next_status_never_claims_a_stale_position(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Playing, 2, 0);
    let (window, _view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert!(next_request(&mut received).is_some());
    let mut acknowledged = initial;
    acknowledged.queue.insert(0, entry(99, CANONICAL, "Aqua"));
    states.send_replace(answered(acknowledged.clone(), 1, Some(99)));
    settle(cx, window);
    assert_eq!(status(cx, window), "Queued next · Aqua");
    let queue = &mut acknowledged.queue;
    let promoted = queue.remove(2);
    queue.insert(0, promoted);
    states.send_replace(acknowledged);
    settle(cx, window);
    assert_eq!(status(cx, window), "", "another entry is now first");
}

#[gpui_kit::test]
fn a_refused_queue_next_keeps_the_text_and_text_typed_since_survives(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Playing, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert!(next_request(&mut received).is_some());
    states.send_replace(answered(initial.clone(), 1, None));
    settle(cx, window);
    assert_eq!(input(cx, &view), VIDEO, "a refusal leaves the text");
    assert!(!status(cx, window).starts_with("Queued next"));
    click(cx, window, "enqueue-next");
    assert_eq!(next_request(&mut received), Some((2, VIDEO.into())));
    type_link(cx, window, "https://youtu.be/bbbbbbbbbbb");
    let mut acknowledged = initial;
    acknowledged.queue.insert(0, entry(99, CANONICAL, "Aqua"));
    states.send_replace(answered(acknowledged, 2, Some(99)));
    settle(cx, window);
    assert_eq!(input(cx, &view), "https://youtu.be/bbbbbbbbbbb");
    assert_eq!(status(cx, window), "Video link recognized.");
}

#[gpui_kit::test]
fn queue_next_is_disabled_offline_or_full_while_add_to_queue_keeps_its_rule(
    cx: &mut TestAppContext,
) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 100, 0));
    assert!(button(cx, window, "enqueue-next").1, "no slot is free");
    assert!(
        !button(cx, window, "enqueue").1,
        "Add to queue is capacity-blind"
    );
    assert_eq!(
        cx.update(|cx| view.read(cx).queue_next_help()),
        "Queue full · remove a track to queue another"
    );
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert!(received.try_recv().is_err());
    click(cx, window, "enqueue");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest {
            placement: crate::player::Placement::End,
            ..
        })
    ));
    states.send_replace(State {
        available: false,
        ..with_lists(Phase::Ready, 2, 0)
    });
    settle(cx, window);
    assert!(button(cx, window, "enqueue-next").1, "offline");
    states.send_replace(with_lists(Phase::Ready, 99, 0));
    settle(cx, window);
    assert!(!button(cx, window, "enqueue-next").1, "one slot is free");
}

#[gpui_kit::test]
fn the_composer_actions_fit_both_native_sizes_in_every_import_state(cx: &mut TestAppContext) {
    for (width, height) in SIZES {
        for import in [
            crate::player::ImportStatus::Ready,
            crate::player::ImportStatus::Importing,
        ] {
            let initial = State {
                import,
                ..with_lists(Phase::Playing, 3, 0)
            };
            let ((window, _, _, _), _) = open_at(cx, initial, width, height);
            let frame = bounds(cx, window, "url");
            let add = bounds(cx, window, "enqueue");
            let next = bounds(cx, window, "enqueue-next");
            let import = bounds(cx, window, "import-playlist");
            assert!(add.right() <= next.left() && next.right() <= import.left());
            assert!(import.right() <= frame.right(), "{width}: import fits");
            assert!(next.left() >= frame.left() && next.top() >= frame.bottom());
            assert!(next.size.width > px(0.) && next.size.height > px(0.));
        }
    }
}

#[gpui_kit::test]
fn repeat_track_copy_appears_only_while_repeat_track_is_on(cx: &mut TestAppContext) {
    let mut initial = with_lists(Phase::Playing, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    let note = "Repeat Track is on; press Next to play it.";
    assert!(
        !cx.update(|cx| view.read(cx).queue_next_help())
            .contains(note)
    );
    initial.preferences.repeat = RepeatMode::Track;
    states.send_replace(initial.clone());
    settle(cx, window);
    assert!(
        cx.update(|cx| view.read(cx).queue_next_help())
            .contains(note)
    );
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert!(next_request(&mut received).is_some());
    initial.queue.insert(0, entry(99, CANONICAL, "Aqua"));
    states.send_replace(answered(initial.clone(), 1, Some(99)));
    settle(cx, window);
    assert_eq!(status(cx, window), format!("Queued next · {note} · Aqua"));
    click(cx, window, ("queue-next", 11_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 2,
            id: 11,
        })
    ));
    let mut moved = initial;
    let item = moved.queue.remove(2);
    moved.queue.insert(0, item);
    states.send_replace(answered(moved, 2, Some(11)));
    settle(cx, window);
    assert_eq!(
        status(cx, window),
        format!("Moved to next · {note} · Queued 11")
    );
}

#[gpui_kit::test]
fn a_long_title_with_repeat_track_on_still_leads_with_the_note(cx: &mut TestAppContext) {
    let mut initial = with_lists(Phase::Playing, 2, 0);
    initial.preferences.repeat = RepeatMode::Track;
    let long_title = "Aqua Marine Orchestra Performs the Complete Extended Live Suite in One Take";
    let (window, _view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    click(cx, window, "enqueue-next");
    assert!(next_request(&mut received).is_some());
    initial.queue.insert(0, entry(99, CANONICAL, long_title));
    states.send_replace(answered(initial, 1, Some(99)));
    settle(cx, window);
    assert!(
        status(cx, window)
            .starts_with("Queued next · Repeat Track is on; press Next to play it. · "),
        "the fixed note leads so only the title truncates"
    );
}

#[gpui_kit::test]
fn search_offers_three_actions_inside_the_window_and_each_sends_its_own(cx: &mut TestAppContext) {
    for (width, height) in SIZES {
        let ((window, _, mut received, states), _) =
            open_at(cx, state(Phase::Playing), width, height);
        type_link(cx, window, "Bach cello");
        act(cx, window, |window, cx| window.press("enter", cx));
        assert!(matches!(
            received.try_recv(),
            Ok(Command::Search { id: 1, .. })
        ));
        states.send_replace(snapshot(Phase::Playing, results()));
        settle(cx, window);
        click(cx, window, "search-row-0");
        let actions = bounds(cx, window, "search-actions");
        let mut previous = None;
        for id in ["search-play", "search-enqueue", "search-enqueue-next"] {
            let rect = bounds(cx, window, id);
            assert!(
                rect.right() <= actions.right() && rect.left() >= actions.left(),
                "{width}: {id}"
            );
            assert!(rect.bottom() <= actions.bottom() && rect.top() >= actions.top());
            assert!(previous.is_none_or(|before: gpui_kit::Pixels| before <= rect.left()));
            previous = Some(rect.right());
        }
        click(cx, window, "search-enqueue-next");
        assert!(matches!(
            received.try_recv(),
            Ok(Command::SearchAction {
                search_id: 1,
                video_id,
                action: SearchAction::Enqueue { id: 1, placement: Placement::Next },
            }) if video_id == "aaaaaaaaaa0"
        ));
        for id in ["search-play", "search-enqueue", "search-enqueue-next"] {
            assert!(button(cx, window, id).1, "{id} waits for the controller");
        }
        click(cx, window, "search-enqueue");
        assert!(received.try_recv().is_err());
    }
}

#[gpui_kit::test]
fn a_search_queue_next_acknowledgement_names_the_entry_and_returns_focus(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Playing, 2, 0));
    type_link(cx, window, "Bach cello");
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(received.try_recv().is_ok());
    let mut open_results = snapshot(Phase::Playing, results());
    open_results.queue = (10..12).map(queue_item).collect();
    states.send_replace(open_results.clone());
    settle(cx, window);
    click(cx, window, "search-row-0");
    click(cx, window, "search-enqueue-next");
    assert!(received.try_recv().is_ok());
    let mut accepted = snapshot(Phase::Playing, SearchPhase::Idle);
    accepted.queue = open_results.queue;
    accepted.queue.insert(
        0,
        entry(42, "https://www.youtube.com/watch?v=aaaaaaaaaa0", "Found"),
    );
    states.send_replace(answered(accepted, 1, Some(42)));
    settle(cx, window);
    cx.update(|cx| assert_eq!(view.read(cx).added, Some((42, Landing::Next))));
    assert_eq!(status(cx, window), "Queued next · Found");
    assert!(focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn a_superseded_search_session_cannot_send_queue_next(cx: &mut TestAppContext) {
    let (window, _view, mut received, states) = open(cx, state(Phase::Playing));
    type_link(cx, window, "Bach cello");
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(received.try_recv().is_ok());
    states.send_replace(snapshot(Phase::Playing, results()));
    settle(cx, window);
    click(cx, window, "search-row-0");
    type_link(cx, window, "something else");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::CancelSearch { id: 1 })
    ));
    assert!(!present(cx, window, "search-enqueue-next"));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn history_menu_lists_queue_next_between_add_and_forget(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Playing, 1, 3));
    show(cx, &view, Section::History);
    click(cx, window, ("history-more", 1_usize));
    let add = bounds(cx, window, ("history-queue", 1_usize));
    let next = bounds(cx, window, ("history-next", 1_usize));
    let forget = bounds(cx, window, ("history-remove", 1_usize));
    assert!(add.bottom() <= next.top() && next.bottom() <= forget.top());
    assert_eq!(
        button(cx, window, ("history-next", 1_usize)).0,
        "Queue next"
    );
    click(cx, window, ("history-next", 1_usize));
    let url = history_entry(1).url;
    assert_eq!(next_request(&mut received), Some((1, url.clone())));
    click(cx, window, ("history-next", 2_usize));
    assert!(
        received.try_recv().is_err(),
        "ignored while a request is pending"
    );
    type_link(cx, window, "unrelated typing");
    let mut acknowledged = with_lists(Phase::Playing, 1, 3);
    acknowledged.queue.insert(0, entry(99, &url, "Played 1"));
    states.send_replace(answered(acknowledged, 1, Some(99)));
    settle(cx, window);
    assert!(
        status(cx, window).is_empty() || status(cx, window).starts_with("Queued next"),
        "typing may hide the line but never wedges later actions"
    );
    cx.update(|cx| assert!(view.read(cx).enqueue_pending.is_none()));
}

#[gpui_kit::test]
fn typing_during_a_history_request_does_not_lose_its_acknowledgement(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Playing, 1, 3));
    show(cx, &view, Section::History);
    click(cx, window, ("history-next", 1_usize));
    let url = history_entry(1).url;
    assert_eq!(next_request(&mut received), Some((1, url.clone())));
    type_link(cx, window, &url);
    cx.update(|cx| assert!(view.read(cx).enqueue_pending.is_some()));
    let mut acknowledged = with_lists(Phase::Playing, 1, 3);
    acknowledged.queue.insert(0, entry(99, &url, "Played 1"));
    states.send_replace(answered(acknowledged, 1, Some(99)));
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        url,
        "list-sourced requests never clear the composer"
    );
    cx.update(|cx| {
        assert!(view.read(cx).enqueue_pending.is_none());
        assert_eq!(view.read(cx).added, Some((99, Landing::Next)));
    });
}

#[gpui_kit::test]
fn history_queue_next_is_unavailable_when_full_but_add_to_queue_still_sends(
    cx: &mut TestAppContext,
) {
    let (window, view, mut received, _states) = open(cx, with_lists(Phase::Ready, 100, 2));
    show(cx, &view, Section::History);
    assert!(button(cx, window, ("history-next", 0_usize)).1);
    click(cx, window, ("history-next", 0_usize));
    assert!(received.try_recv().is_err());
    click(cx, window, ("history-queue", 0_usize));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest {
            placement: crate::player::Placement::End,
            ..
        })
    ));
}

#[gpui_kit::test]
fn move_to_next_is_off_for_the_first_row_and_on_for_a_full_queue(cx: &mut TestAppContext) {
    let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Playing, 100, 0));
    assert!(
        button(cx, window, ("queue-next", 10_u64)).1,
        "row 0 is already next"
    );
    click(cx, window, ("queue-next", 10_u64));
    assert!(received.try_recv().is_err());
    assert!(
        !button(cx, window, ("queue-next", 11_u64)).1,
        "never disabled by a full queue"
    );
    click(cx, window, ("queue-next", 11_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 1,
            id: 11
        })
    ));
    click(cx, window, ("queue-next", 12_u64));
    assert!(received.try_recv().is_err(), "one request at a time");
}

#[gpui_kit::test]
fn move_to_next_confirms_only_after_the_controller_and_only_while_first(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Playing, 4, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    click(cx, window, ("queue-next", 13_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 1,
            id: 13
        })
    ));
    assert_eq!(status(cx, window), "");
    let mut moved = initial;
    let item = moved.queue.remove(3);
    moved.queue.insert(0, item);
    states.send_replace(answered(moved.clone(), 1, Some(13)));
    settle(cx, window);
    assert_eq!(status(cx, window), "Moved to next · Queued 13");
    cx.update(|cx| assert_eq!(view.read(cx).added, Some((13, Landing::Moved))));
    let item = moved.queue.remove(1);
    moved.queue.insert(0, item);
    states.send_replace(moved);
    settle(cx, window);
    assert_eq!(status(cx, window), "");
}

#[gpui_kit::test]
fn a_stale_move_is_silent_and_releases_the_next_request(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Playing, 4, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    click(cx, window, ("queue-next", 12_u64));
    assert!(received.try_recv().is_ok());
    states.send_replace(answered(initial, 1, None));
    settle(cx, window);
    assert_eq!(status(cx, window), "");
    cx.update(|cx| {
        assert!(view.read(cx).enqueue_pending.is_none());
        assert!(view.read(cx).added.is_none());
    });
    click(cx, window, ("queue-next", 12_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 2,
            id: 12
        })
    ));
}

#[gpui_kit::test]
fn the_queue_menu_moves_to_next_from_the_keyboard_alone(cx: &mut TestAppContext) {
    let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Playing, 4, 0));
    tab_until(cx, window, ("queue-more", 12_u64));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, ("queue-next", 12_u64)));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext {
            request_id: 1,
            id: 12
        })
    ));
}

#[gpui_kit::test]
fn the_history_menu_queues_next_from_the_keyboard_alone(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, with_lists(Phase::Playing, 1, 3));
    show(cx, &view, Section::History);
    tab_until(cx, window, ("history-more", 0_usize));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, ("history-queue", 0_usize)));
    act(cx, window, |window, cx| window.press("down", cx));
    assert!(focused(cx, window, ("history-next", 0_usize)));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(next_request(&mut received), Some((1, history_entry(0).url)));
}

#[gpui_kit::test]
fn the_composer_button_works_from_the_keyboard_alone(cx: &mut TestAppContext) {
    let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Playing, 1, 0));
    type_link(cx, window, VIDEO);
    tab_until(cx, window, "enqueue-next");
    act(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(next_request(&mut received), Some((1, VIDEO.into())));
}

#[gpui_kit::test]
fn a_deep_row_moved_to_next_is_revealed_and_keeps_focus(cx: &mut TestAppContext) {
    let ((window, view, mut received, states), _) =
        open_at(cx, with_lists(Phase::Playing, 60, 0), 420., 480.);
    cx.update(|cx| {
        view.update(cx, |app, _| {
            app.queue_scroll.state.scroll_to_reveal_item(40);
        });
    });
    settle(cx, window);
    let deep = 50_u64;
    click(cx, window, ("queue-next", deep));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::MoveToNext { request_id: 1, id }) if id == deep
    ));
    settle(cx, window);
    assert!(focused(cx, window, ("queue-more", deep)));
    let mut moved = with_lists(Phase::Playing, 60, 0);
    let item = moved.queue.remove(40);
    moved.queue.insert(0, item);
    states.send_replace(answered(moved, 1, Some(deep)));
    settle(cx, window);
    settle(cx, window);
    let list = bounds(cx, window, "queue-list");
    let row = bounds(cx, window, ("queue-row", deep));
    assert!(
        row.top() >= list.top() && row.bottom() <= list.bottom(),
        "row 0 is visible"
    );
    assert!(focused(cx, window, ("queue-more", deep)));
}
