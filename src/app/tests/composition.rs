//! Main-player composition: composer recognition and confirmed-add feedback, queue row
//! keyboard model, focus after removal, and the state-specific skeletons.
use super::animation_fixes::focused;
use super::*;
use crate::app::composer::LinkKind;

const CANONICAL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const VIDEO: &str = "https://youtu.be/aqz-KE-bpKQ";
const PLAYLIST_WATCH: &str =
    "https://www.youtube.com/watch?v=aqz-KE-bpKQ&list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG";
const PLAYLIST: &str = "https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG";

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

fn input(cx: &TestAppContext, view: &Entity<PlayerApp>) -> String {
    cx.update(|cx| view.read(cx).input.read(cx).value().to_string())
}

fn type_link(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.press("ctrl-a", cx);
        window.input(text, cx);
    });
}

fn tab_until(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    for _ in 0..40 {
        if focused(cx, window, id.clone()) {
            return;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
    }
    assert!(
        focused(cx, window, id.clone()),
        "keyboard focus never reached {id:?}"
    );
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

#[test]
fn links_are_recognized_locally() {
    assert_eq!(LinkKind::of(""), LinkKind::Empty);
    assert_eq!(LinkKind::of("   "), LinkKind::Empty);
    assert_eq!(LinkKind::of(VIDEO), LinkKind::Video);
    assert_eq!(LinkKind::of(PLAYLIST_WATCH), LinkKind::VideoInPlaylist);
    assert_eq!(LinkKind::of(PLAYLIST), LinkKind::Playlist);
    assert_eq!(LinkKind::of("not a link"), LinkKind::Other);
    assert_eq!(
        LinkKind::of("https://example.com/watch?v=aqz-KE-bpKQ"),
        LinkKind::Other
    );
}

#[gpui_kit::test]
fn pasting_a_link_only_describes_it(cx: &mut TestAppContext) {
    let (window, view, mut received, _) = open(cx, state(Phase::Ready));
    assert_eq!(status(cx, window), "");
    type_link(cx, window, VIDEO);
    assert_eq!(status(cx, window), "Video link recognized.");
    type_link(cx, window, PLAYLIST_WATCH);
    assert!(status(cx, window).starts_with("Video in a playlist"));
    type_link(cx, window, PLAYLIST);
    assert!(status(cx, window).starts_with("Playlist link"));
    type_link(cx, window, "half-typed");
    assert_eq!(status(cx, window), "", "unrecognized text is never scolded");
    assert_eq!(input(cx, &view), "half-typed", "the text is untouched");
    assert!(
        received.try_recv().is_err(),
        "recognition never plays or imports"
    );
}

#[gpui_kit::test]
fn the_composer_is_one_frame_with_play_attached(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, _, _, _), _) = open_at(cx, state(Phase::Ready), width, height);
        let frame = bounds(cx, window, "url");
        let play = bounds(cx, window, "play");
        let editor = bounds(cx, window, "url-editor");
        assert!(play.left() > frame.left() && play.right() <= frame.right());
        assert!(play.top() >= frame.top() && play.bottom() <= frame.bottom());
        assert!(
            editor.size.width >= px(width - 32. - 140.),
            "{width}: a long link keeps at least {} px to edit in",
            width - 32. - 140.
        );
        assert!(!present(cx, window, "url-more"), "no one-item More menu");
        assert!(
            present(cx, window, "import-playlist"),
            "import stays visible"
        );
        let add = bounds(cx, window, "enqueue");
        assert!(
            add.top() >= frame.bottom(),
            "queue and import actions sit under the frame"
        );
    }
}

#[gpui_kit::test]
fn an_empty_idle_player_has_no_headline_or_transport_but_keeps_footer_volume(
    cx: &mut TestAppContext,
) {
    let (window, _, _, _) = open(cx, state(Phase::Ready));
    for id in ["title", "pause-resume", "stop", "next", "seek-status"] {
        assert!(
            !present(cx, window, id),
            "{id} must not appear in an empty idle player"
        );
    }
    assert!(present(cx, window, "play") && present(cx, window, "queue-empty"));
    // Volume stays settable before anything plays, in the same right-hand slot as elsewhere.
    let volume = bounds(cx, window, "volume-percent");
    assert!(volume.right() <= px(520. - 16. + 1.) && volume.right() >= px(520. - 16. - 1.));
    let footer = bounds(cx, window, "audio-footer");
    assert!(volume.top() >= footer.top() && volume.bottom() <= footer.bottom());
}

#[gpui_kit::test]
fn an_idle_pending_queue_starts_above_the_collection_with_footer_volume(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, with_lists(Phase::Ready, 3, 0));
    assert!(!present(cx, window, "title"));
    let start = bounds(cx, window, "next");
    assert!(
        start.size.width < px(160.),
        "Play Queue is not a full-width bar"
    );
    assert!(bounds(cx, window, "volume-percent").top() > start.bottom());
    click(cx, window, "next");
    assert!(matches!(received.try_recv(), Ok(Command::Next)));
}

#[gpui_kit::test]
fn a_confirmed_add_clears_the_field_and_is_acknowledged(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 2, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(url)) if url == VIDEO));
    // Nothing is claimed before the controller answers; the text stays and a repeat is ignored.
    assert_eq!(input(cx, &view), VIDEO);
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(
        received.try_recv().is_err(),
        "a repeat of the unanswered text is ignored"
    );
    states.send_replace(State {
        position: Duration::from_secs(1),
        ..initial.clone()
    });
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        VIDEO,
        "an unrelated snapshot is not an acknowledgement"
    );
    let mut acknowledged = initial;
    acknowledged.queue.push(QueueItem {
        id: 99,
        url: CANONICAL.into(),
        title: Some("Aqua".into()),
        title_unavailable: false,
    });
    states.send_replace(acknowledged);
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        "",
        "the submitted text is cleared on confirmation"
    );
    assert_eq!(status(cx, window), "Added to queue · #3 · Aqua");
    cx.update(|cx| assert_eq!(view.read(cx).added, Some(99)));
    // Typing starts a new entry and retires the confirmation.
    type_link(cx, window, "x");
    assert_eq!(status(cx, window), "");
}

#[gpui_kit::test]
fn a_refused_add_keeps_the_text_and_can_be_retried(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 1, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, "https://youtu.be/zzzzzzzzzzz");
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    states.send_replace(State {
        error: Some("Could not add that link.".into()),
        ..initial
    });
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        "https://youtu.be/zzzzzzzzzzz",
        "validation failure keeps the text"
    );
    cx.update(|cx| assert!(view.read(cx).added.is_none()));
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(
        matches!(received.try_recv(), Ok(Command::Enqueue(_))),
        "retry is allowed after a refusal"
    );
}

#[gpui_kit::test]
fn text_typed_after_submitting_survives_the_acknowledgement(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 1, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    act(cx, window, |window, cx| window.input("&t=30", cx));
    let mut acknowledged = initial;
    acknowledged.queue.push(QueueItem {
        id: 99,
        url: CANONICAL.into(),
        title: None,
        title_unavailable: false,
    });
    states.send_replace(acknowledged);
    settle(cx, window);
    assert_eq!(input(cx, &view), format!("{VIDEO}&t=30"));
}

#[gpui_kit::test]
fn adding_nothing_says_so_locally(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, state(Phase::Ready));
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(received.try_recv().is_err(), "an empty field sends nothing");
    assert_eq!(status(cx, window), "Paste a link first.");
}

#[gpui_kit::test]
fn the_status_line_never_moves_the_queue(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, _, _, _), _) = open_at(cx, with_lists(Phase::Playing, 6, 0), width, height);
        let before = bounds(cx, window, "queue-region");
        type_link(cx, window, PLAYLIST);
        assert_eq!(
            bounds(cx, window, "queue-region"),
            before,
            "{width}: recognition shifts nothing"
        );
        act(cx, window, |window, cx| window.press("ctrl-enter", cx));
        assert_eq!(
            bounds(cx, window, "queue-region"),
            before,
            "{width}: feedback shifts nothing"
        );
    }
}

#[gpui_kit::test]
fn alt_arrows_and_delete_act_on_the_focused_entry(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, with_lists(Phase::Ready, 3, 0));
    tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-up", cx));
    assert!(
        received.try_recv().is_err(),
        "the first entry cannot move up"
    );
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, up: false })
    ));
    act(cx, window, |window, cx| window.press("delete", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Dequeue(10))));
    tab_until(cx, window, ("queue-more", 12_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(
        received.try_recv().is_err(),
        "the last entry cannot move down"
    );
    act(cx, window, |window, cx| window.press("alt-up", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 12, up: true })
    ));
}

#[gpui_kit::test]
fn removing_the_focused_entry_keeps_focus_in_the_list(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 3, 0);
    let (window, _, mut received, states) = open(cx, initial.clone());
    tab_until(cx, window, ("queue-more", 11_u64));
    act(cx, window, |window, cx| window.press("delete", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Dequeue(11))));
    let mut after = initial;
    after.queue.remove(1);
    states.send_replace(after.clone());
    settle(cx, window);
    assert!(
        focused(cx, window, ("queue-more", 12_u64)),
        "focus lands on the entry that took its place"
    );
    // Removing the last entry moves focus to the new last one.
    act(cx, window, |window, cx| window.press("delete", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Dequeue(12))));
    after.queue.remove(1);
    states.send_replace(after.clone());
    settle(cx, window);
    assert!(focused(cx, window, ("queue-more", 10_u64)));
    // An emptied queue hands focus back to the link field.
    act(cx, window, |window, cx| window.press("delete", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Dequeue(10))));
    after.queue.clear();
    states.send_replace(after);
    settle(cx, window);
    assert!(focused(cx, window, "url-editor"));
}

#[gpui_kit::test]
fn a_moved_entry_keeps_keyboard_focus(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 3, 0);
    let (window, _, mut received, states) = open(cx, initial.clone());
    tab_until(cx, window, ("queue-more", 10_u64));
    act(cx, window, |window, cx| window.press("alt-down", cx));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, up: false })
    ));
    let mut after = initial;
    after.queue.swap(0, 1);
    states.send_replace(after);
    settle(cx, window);
    assert!(
        focused(cx, window, ("queue-more", 10_u64)),
        "focus follows the entry, not the slot"
    );
}

#[gpui_kit::test]
fn the_next_entry_is_marked_and_the_empty_queue_is_calm(cx: &mut TestAppContext) {
    let (window, view, _, _) = open(cx, with_lists(Phase::Playing, 3, 0));
    let first = bounds(cx, window, ("queue-title", 10_u64));
    let second = bounds(cx, window, ("queue-title", 11_u64));
    assert!(second.top() > first.top());
    cx.update(|cx| {
        assert_eq!(view.read(cx).state.queue.len(), 3);
    });
    let (window, _, _, _) = open(cx, with_lists(Phase::Playing, 0, 0));
    let mut empty = String::new();
    act(cx, window, |window, _| {
        empty = window
            .find("queue-empty")
            .label()
            .unwrap_or_default()
            .to_owned();
    });
    assert!(
        !empty.to_lowercase().contains("paste"),
        "the empty queue does not repeat the composer: {empty}"
    );
}

#[gpui_kit::test]
fn both_native_sizes_keep_the_three_regions_ordered(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        for initial in [
            with_lists(Phase::Playing, 6, 3),
            with_lists(Phase::Paused, 0, 0),
            with_lists(Phase::Ready, 4, 2),
            state(Phase::Ready),
        ] {
            let ((window, _, _, _), _) = open_at(cx, initial, width, height);
            let header = bounds(cx, window, "player-header");
            let composer = bounds(cx, window, "link-entry");
            let queue = bounds(cx, window, "queue-region");
            assert!(header.bottom() <= composer.top() && composer.bottom() <= queue.top());
            assert!(
                composer.bottom() + px(8.) < queue.top() + px(1.),
                "{width}: regions are separated"
            );
            assert!(queue.bottom() <= px(height - 16. + 1.));
            assert!(
                queue.size.height >= px(96.),
                "{width}: the queue keeps room for rows"
            );
        }
    }
}

#[gpui_kit::test]
fn another_command_retires_the_added_line(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 1, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    let mut acknowledged = initial;
    acknowledged.queue.push(QueueItem {
        id: 99,
        url: CANONICAL.into(),
        title: Some("Aqua".into()),
        title_unavailable: false,
    });
    states.send_replace(acknowledged);
    settle(cx, window);
    assert!(status(cx, window).starts_with("Added to queue"));
    click(cx, window, "next");
    assert!(matches!(received.try_recv(), Ok(Command::Next)));
    assert_eq!(status(cx, window), "");
    cx.update(|cx| assert!(view.read(cx).added.is_none()));
}

#[gpui_kit::test]
fn only_the_matching_entry_confirms_an_add(cx: &mut TestAppContext) {
    let initial = with_lists(Phase::Ready, 1, 0);
    let (window, view, mut received, states) = open(cx, initial.clone());
    type_link(cx, window, VIDEO);
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    // A playlist import or History add lands first: someone else's entry.
    let mut other = initial;
    other.queue.push(QueueItem {
        id: 50,
        url: "https://www.youtube.com/watch?v=bbbbbbbbbbb".into(),
        title: Some("Someone else".into()),
        title_unavailable: false,
    });
    states.send_replace(other.clone());
    settle(cx, window);
    assert_eq!(
        input(cx, &view),
        VIDEO,
        "the text is kept until this request is answered"
    );
    assert_eq!(status(cx, window), "Video link recognized.");
    cx.update(|cx| assert!(view.read(cx).added.is_none()));
    // Then the controller refuses the Enqueue: the text must still be there.
    other.error = Some("Could not add that link.".into());
    states.send_replace(other.clone());
    settle(cx, window);
    assert_eq!(input(cx, &view), VIDEO);
    // A retry that is accepted is confirmed by its own entry only.
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    other.queue.push(QueueItem {
        id: 51,
        url: CANONICAL.into(),
        title: Some("Aqua".into()),
        title_unavailable: false,
    });
    states.send_replace(other);
    settle(cx, window);
    assert_eq!(input(cx, &view), "");
    cx.update(|cx| assert_eq!(view.read(cx).added, Some(51)));
}

#[gpui_kit::test]
fn focus_survives_moves_to_both_boundaries_and_shortcuts_still_work(cx: &mut TestAppContext) {
    for up in [true, false] {
        let initial = with_lists(Phase::Ready, 3, 0);
        let (window, _, mut received, states) = open(cx, initial.clone());
        tab_until(cx, window, ("queue-more", 11_u64));
        act(cx, window, |window, cx| {
            window.press(if up { "alt-up" } else { "alt-down" }, cx);
        });
        assert!(
            matches!(received.try_recv(), Ok(Command::Shift { id: 11, up: sent }) if sent == up)
        );
        let mut after = initial;
        after.queue.swap(1, if up { 0 } else { 2 });
        states.send_replace(after);
        settle(cx, window);
        assert!(
            focused(cx, window, ("queue-more", 11_u64)),
            "stable trigger retains usable focus at the boundary"
        );
        act(cx, window, |window, cx| {
            window.press(if up { "alt-up" } else { "alt-down" }, cx);
        });
        assert!(received.try_recv().is_err(), "boundary move is ignored");
        act(cx, window, |window, cx| window.press("tab", cx));
        assert!(
            !focused(cx, window, ("queue-more", 11_u64)),
            "Tab is not stranded"
        );
    }
}
