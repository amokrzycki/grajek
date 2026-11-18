use super::animation_fixes::focused;
use super::composition::tab_until;
use super::search::{results, settle, snapshot};
use super::*;
use crate::player::{FavoriteRefusal, FavoriteResult, Placement, SearchAction};
use crate::store::FavoriteEntry;

const VIDEO: &str = "https://www.youtube.com/watch?v=aaaaaaaaaaa";

fn favorite(id: &str, title: Option<&str>) -> FavoriteEntry {
    FavoriteEntry {
        url: format!("https://www.youtube.com/watch?v={id}"),
        title: title.map(str::to_owned),
        channel: None,
        duration_secs: Some(225),
        live: false,
    }
}

fn with_favorites(phase: Phase, favorites: Vec<FavoriteEntry>) -> State {
    State {
        favorites,
        ..state(phase)
    }
}

fn playing_url(phase: Phase, url: &str, title: Option<&str>) -> State {
    State {
        current_url: Some(url.to_owned()),
        ..playing(phase, title, 10)
    }
}

fn ack(mut state: State, request_id: u64, url: &str, saved: bool) -> State {
    state.favorite_result = Some(FavoriteResult {
        request_id,
        url: url.to_owned(),
        saved,
        result: Ok(()),
    });
    state
}

fn open_favorite_menu(cx: &mut TestAppContext, window: AnyWindowHandle, url: &str) {
    let trigger = format!("favorite-more:{url}");
    if present(cx, window, "track-options-menu") {
        return;
    }
    act(cx, window, |window, cx| {
        window.click(trigger, cx);
        window.render_frame(cx);
    });
}

fn favorite_menu(cx: &mut TestAppContext, window: AnyWindowHandle, url: &str, action: &str) {
    open_favorite_menu(cx, window, url);
    click(cx, window, format!("{action}:{url}"));
}

fn submit_search(cx: &mut TestAppContext, window: AnyWindowHandle, received: &mut TestCommands) {
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.press("ctrl-a", cx);
        window.input("Bach cello", cx);
        window.press("enter", cx);
    });
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Search { id: 1, .. })
    ));
}

#[gpui_kit::test]
fn current_track_save_and_remove_target_the_exact_url_without_optimism(cx: &mut TestAppContext) {
    let (window, view, mut received, states) =
        open(cx, playing_url(Phase::Playing, VIDEO, Some("Song")));
    assert_eq!(
        button(cx, window, "now-playing-favorite"),
        ("Save to Favorites: Song".into(), false)
    );
    click(cx, window, "now-playing-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 1, url, saved: true }) if url == VIDEO
    ));
    assert!(
        !present(cx, window, "now-playing-saved"),
        "no optimistic membership"
    );
    click(cx, window, "now-playing-favorite");
    assert!(
        received.try_recv().is_err(),
        "a pending request blocks a repeat"
    );
    let mut confirmed = playing_url(Phase::Playing, VIDEO, Some("Song"));
    confirmed.favorites = vec![favorite("aaaaaaaaaaa", Some("Song"))];
    states.send_replace(ack(confirmed, 1, VIDEO, true));
    cx.run_until_parked();
    assert_eq!(
        button(cx, window, "now-playing-favorite"),
        ("Remove from Favorites: Song".into(), false)
    );
    assert!(present(cx, window, "now-playing-saved"));
    click(cx, window, "now-playing-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 2, url, saved: false }) if url == VIDEO
    ));
    cx.update(|cx| assert!(view.read(cx).favorite_pending.is_some()));
}

#[gpui_kit::test]
fn save_is_available_while_joining_and_preparing(cx: &mut TestAppContext) {
    for phase in [Phase::Connecting, Phase::Resolving] {
        let (window, _, mut received, _) = open(
            cx,
            State {
                current_url: Some(VIDEO.to_owned()),
                ..state(phase)
            },
        );
        assert!(present(cx, window, "now-playing-favorite"));
        click(cx, window, "now-playing-favorite");
        assert!(matches!(
            received.try_recv(),
            Ok(Command::SetFavorite { saved: true, url, .. }) if url == VIDEO
        ));
    }
    let (window, _, _, _) = open(cx, state(Phase::Ready));
    assert!(
        !present(cx, window, "now-playing-favorite"),
        "no identity, no control"
    );
}

#[gpui_kit::test]
fn save_remains_available_without_discord_but_not_without_a_controller(cx: &mut TestAppContext) {
    let offline = State {
        available: false,
        current_url: Some(VIDEO.to_owned()),
        ..playing(Phase::Playing, Some("Song"), 10)
    };
    let (window, _, mut received, _) = open(cx, offline);
    assert_eq!(
        button(cx, window, "now-playing-favorite"),
        ("Save to Favorites: Song".into(), false)
    );
    click(cx, window, "now-playing-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { saved: true, .. })
    ));
    let no_controller = State {
        controller_ready: false,
        current_url: Some(VIDEO.to_owned()),
        ..playing(Phase::Playing, Some("Song"), 10)
    };
    let (window, _, mut received, _) = open(cx, no_controller);
    assert!(button(cx, window, "now-playing-favorite").1);
    click(cx, window, "now-playing-favorite");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn queue_and_history_menus_save_and_remove_by_canonical_url(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 1, 1));
    let queue_url = queue_item(10).url;
    click(cx, window, ("queue-favorite", 10_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 1, url, saved: true }) if url == queue_url
    ));
    let mut confirmed = with_lists(Phase::Ready, 1, 1);
    confirmed.favorites = vec![FavoriteEntry {
        url: queue_url.clone(),
        title: Some("Queued 10".into()),
        ..favorite("bbbbbbbbbbb", None)
    }];
    states.send_replace(ack(confirmed, 1, &queue_url, true));
    cx.run_until_parked();
    assert_eq!(
        button(cx, window, ("queue-favorite", 10_u64)).0,
        "Remove from Favorites"
    );
    assert!(present(cx, window, ("queue-saved", 10_u64)));
    show(cx, &view, Section::History);
    let history_url = history_entry(0).url;
    click(cx, window, ("history-favorite", 0_usize));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 2, url, saved: true }) if url == history_url
    ));
}

#[gpui_kit::test]
fn list_insertions_share_request_ids_and_wait_for_acknowledgement(cx: &mut TestAppContext) {
    let snapshot = State {
        favorites: vec![favorite("aaaaaaaaaaa", Some("Saved"))],
        ..with_lists(Phase::Ready, 0, 1)
    };
    let (window, view, mut received, states) = open(cx, snapshot.clone());
    show(cx, &view, Section::Favorites);
    let url = snapshot.favorites[0].url.clone();
    favorite_menu(cx, window, &url, "favorite-queue");
    assert!(matches!(
        received.receiver.try_recv(),
        Ok(Command::EnqueueRequest { id: 1, value, placement: Placement::End }) if value == url
    ));
    favorite_menu(cx, window, &url, "favorite-next");
    assert!(received.receiver.try_recv().is_err());
    let mut accepted = snapshot.clone();
    accepted.enqueue_result = Some(crate::player::EnqueueResult {
        request_id: 1,
        item_id: Some(99),
    });
    states.send_replace(accepted);
    cx.run_until_parked();
    favorite_menu(cx, window, &url, "favorite-next");
    assert!(matches!(
        received.receiver.try_recv(),
        Ok(Command::EnqueueRequest { id: 2, value, placement: Placement::Next }) if value == url
    ));
    let mut refused = snapshot;
    refused.enqueue_result = Some(crate::player::EnqueueResult {
        request_id: 2,
        item_id: None,
    });
    states.send_replace(refused);
    cx.run_until_parked();
    show(cx, &view, Section::History);
    click(cx, window, ("history-queue", 0_usize));
    assert!(matches!(
        received.receiver.try_recv(),
        Ok(Command::EnqueueRequest { id: 3, value, placement: Placement::End }) if value == history_entry(0).url
    ));
}

#[gpui_kit::test]
fn favorite_rows_play_add_next_and_remove_with_exact_commands(cx: &mut TestAppContext) {
    let mut snapshot = State {
        favorites: vec![
            favorite("aaaaaaaaaaa", Some("Saved")),
            favorite("bbbbbbbbbbb", None),
        ],
        ..with_lists(Phase::Playing, 2, 0)
    };
    snapshot.track = playing(Phase::Playing, Some("Now"), 5).track;
    snapshot.track_id = 1;
    let (window, view, mut received, states) = open(cx, snapshot.clone());
    show(cx, &view, Section::Favorites);
    let url = favorite("aaaaaaaaaaa", Some("Saved")).url;
    click(cx, window, format!("favorite-play:{url}"));
    assert!(matches!(received.try_recv(), Ok(Command::Play(sent)) if sent == url));
    favorite_menu(cx, window, &url, "favorite-queue");
    assert!(matches!(
        received.receiver.try_recv(),
        Ok(Command::EnqueueRequest { id: 1, value, placement: Placement::End }) if value == url
    ));
    snapshot.enqueue_result = Some(crate::player::EnqueueResult {
        request_id: 1,
        item_id: None,
    });
    states.send_replace(snapshot);
    cx.run_until_parked();
    favorite_menu(cx, window, &url, "favorite-next");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::EnqueueRequest { id: 2, value, placement: Placement::Next }) if value == url
    ));
    favorite_menu(cx, window, &url, "favorite-remove");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { url: sent, saved: false, .. }) if sent == url
    ));
}

#[gpui_kit::test]
fn favorite_row_click_never_autoplays(cx: &mut TestAppContext) {
    let snapshot = State {
        favorites: vec![favorite("aaaaaaaaaaa", Some("Saved"))],
        ..with_lists(Phase::Ready, 0, 0)
    };
    let (window, view, mut received, _) = open(cx, snapshot);
    show(cx, &view, Section::Favorites);
    let url = favorite("aaaaaaaaaaa", Some("Saved")).url;
    click(cx, window, format!("favorite-title:{url}"));
    assert!(received.try_recv().is_err(), "row click is not Play");
}

#[gpui_kit::test]
fn full_favorites_refusal_surfaces_the_refusal_text(cx: &mut TestAppContext) {
    let (window, view, mut received, states) =
        open(cx, playing_url(Phase::Playing, VIDEO, Some("Song")));
    click(cx, window, "now-playing-favorite");
    assert!(received.try_recv().is_ok());
    let mut refused = playing_url(Phase::Playing, VIDEO, Some("Song"));
    refused.favorite_result = Some(FavoriteResult {
        request_id: 1,
        url: VIDEO.to_owned(),
        saved: true,
        result: Err(FavoriteRefusal::Full),
    });
    states.send_replace(refused);
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).favorite_pending.is_none()));
    let text = cx
        .update_window(window, |_, window, _| {
            window
                .find("link-status")
                .label()
                .unwrap_or_default()
                .to_owned()
        })
        .ok();
    assert_eq!(
        text.as_deref(),
        Some("Favorites full (500 tracks). Remove a favorite and try again.")
    );
    assert_eq!(
        button(cx, window, "now-playing-favorite"),
        ("Save to Favorites: Song".into(), false),
        "refusal changes nothing"
    );
}

#[gpui_kit::test]
fn delayed_acknowledgement_and_saturated_channel_do_not_claim_membership(cx: &mut TestAppContext) {
    let (window, view, mut received, _) =
        open(cx, playing_url(Phase::Playing, VIDEO, Some("Song")));
    cx.update(|cx| {
        for _ in 0..16 {
            assert!(view.read(cx).send(Command::CancelImport));
        }
    });
    click(cx, window, "now-playing-favorite");
    assert!(
        !present(cx, window, "now-playing-saved"),
        "saturated send is not optimistic"
    );
    for _ in 0..16 {
        assert!(matches!(received.try_recv(), Ok(Command::CancelImport)));
    }
    assert!(received.try_recv().is_err());
    click(cx, window, "now-playing-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite {
            id: 1,
            saved: true,
            ..
        })
    ));
    cx.update(|cx| assert!(view.read(cx).favorite_pending.is_some()));
}

#[gpui_kit::test]
fn insertion_reveals_the_same_favorite_at_the_scrolled_end(cx: &mut TestAppContext) {
    let mut initial = playing_url(Phase::Playing, VIDEO, Some("Current"));
    initial.favorites = (0..20)
        .map(|id| favorite(&format!("{id:011}"), Some("Saved")))
        .collect();
    let ((window, view, _, states), _) = open_at(cx, initial.clone(), 420., 480.);
    show(cx, &view, Section::Favorites);
    tab_until(
        cx,
        window,
        format!("favorite-more:{}", initial.favorites[0].url),
    );
    act(cx, window, |window, cx| window.press("end", cx));
    let url = initial.favorites[19].url.clone();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, format!("favorite-more:{url}")));
    initial
        .favorites
        .insert(0, favorite("zzzzzzzzzzz", Some("Inserted")));
    states.send_replace(initial);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, format!("favorite-more:{url}")));
    act(cx, window, |window, _| {
        let row = window.find(format!("favorite-row:{url}")).bounds();
        let list = window.find("favorites-list").bounds();
        assert!(
            row.top() >= list.top() && row.bottom() <= list.bottom(),
            "focused favorite must remain visible: {row:?} outside {list:?}"
        );
    });
}

#[gpui_kit::test]
fn insertion_and_removal_preserve_url_focus(cx: &mut TestAppContext) {
    let initial = State {
        favorites: vec![
            favorite("aaaaaaaaaaa", Some("A")),
            favorite("bbbbbbbbbbb", Some("B")),
        ],
        ..with_lists(Phase::Ready, 0, 0)
    };
    let (window, view, _, states) = open(cx, initial.clone());
    show(cx, &view, Section::Favorites);
    let url_b = favorite("bbbbbbbbbbb", Some("B")).url;
    tab_until(cx, window, format!("favorite-more:{url_b}"));
    let mut inserted = initial.clone();
    inserted
        .favorites
        .insert(0, favorite("ccccccccccc", Some("C")));
    states.send_replace(inserted);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, format!("favorite-more:{url_b}")));
    let mut removed = initial;
    removed.favorites.remove(1);
    states.send_replace(removed);
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(
        cx,
        window,
        format!("favorite-more:{}", favorite("aaaaaaaaaaa", Some("A")).url)
    ));
}

#[gpui_kit::test]
fn removing_the_last_favorite_returns_focus_to_the_composer(cx: &mut TestAppContext) {
    let initial = State {
        favorites: vec![favorite("aaaaaaaaaaa", Some("A"))],
        ..with_lists(Phase::Ready, 0, 0)
    };
    let (window, view, mut received, states) = open(cx, initial.clone());
    show(cx, &view, Section::Favorites);
    let url = favorite("aaaaaaaaaaa", Some("A")).url;
    favorite_menu(cx, window, &url, "favorite-remove");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { saved: false, .. })
    ));
    states.send_replace(State {
        favorites: Vec::new(),
        ..initial
    });
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, "url-editor"));
    assert!(present(cx, window, "favorites-empty"));
}

#[gpui_kit::test]
fn full_queue_disables_queue_next_but_keeps_add_and_remove(cx: &mut TestAppContext) {
    let snapshot = State {
        favorites: vec![favorite("aaaaaaaaaaa", Some("A"))],
        ..with_lists(Phase::Ready, 100, 0)
    };
    let (window, view, _, _) = open(cx, snapshot);
    show(cx, &view, Section::Favorites);
    let url = favorite("aaaaaaaaaaa", Some("A")).url;
    open_favorite_menu(cx, window, &url);
    assert!(button(cx, window, format!("favorite-next:{url}")).1);
    assert!(!button(cx, window, format!("favorite-queue:{url}")).1);
    assert!(!button(cx, window, format!("favorite-remove:{url}")).1);
}

#[gpui_kit::test]
fn search_membership_requires_a_controller_but_not_discord(cx: &mut TestAppContext) {
    let (window, _, mut received, states) = open(cx, state(Phase::Ready));
    submit_search(cx, window, &mut received);
    let mut found = snapshot(Phase::Ready, results());
    found.available = false;
    states.send_replace(found.clone());
    settle(cx, window);
    click(cx, window, "search-row-0");
    assert!(!button(cx, window, "search-favorite").1);
    found.controller_ready = false;
    states.send_replace(found);
    settle(cx, window);
    assert!(button(cx, window, "search-favorite").1);
    click(cx, window, "search-favorite");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn search_save_and_remove_use_the_envelope_and_retain_results(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, state(Phase::Ready));
    submit_search(cx, window, &mut received);
    assert!(
        button(cx, window, "search-favorite").1,
        "disabled until selected"
    );
    states.send_replace(snapshot(Phase::Ready, results()));
    settle(cx, window);
    click(cx, window, "search-row-0");
    assert!(!button(cx, window, "search-favorite").1);
    click(cx, window, "search-favorite");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SearchAction { search_id: 1, video_id, action: SearchAction::Favorite { id: 1, saved: true } }) if video_id == "aaaaaaaaaa0"
    ));
    assert!(present(cx, window, "search-row-1"), "results remain");
    assert_eq!(
        cx.update(|cx| view.read(cx).input.read(cx).value().to_string()),
        "Bach cello"
    );
    let mut confirmed = snapshot(Phase::Ready, results());
    confirmed.favorites = vec![favorite("aaaaaaaaaa0", Some("Found"))];
    states.send_replace(ack(
        confirmed,
        1,
        "https://www.youtube.com/watch?v=aaaaaaaaaa0",
        true,
    ));
    settle(cx, window);
    assert!(
        button(cx, window, "search-favorite")
            .0
            .starts_with("Remove from Favorites:"),
        "membership flips the label"
    );
    assert!(present(cx, window, "search-saved"));
    assert!(
        present(cx, window, "search-row-0"),
        "ack keeps results open"
    );
}

#[gpui_kit::test]
fn favorite_menu_is_keyboard_reachable_and_escape_returns_to_the_trigger(cx: &mut TestAppContext) {
    let snapshot = State {
        favorites: vec![favorite("aaaaaaaaaaa", Some("A"))],
        ..with_lists(Phase::Ready, 0, 0)
    };
    let (window, view, mut received, _) = open(cx, snapshot);
    show(cx, &view, Section::Favorites);
    let url = favorite("aaaaaaaaaaa", Some("A")).url;
    let more = format!("favorite-more:{url}");
    for _ in 0..16 {
        if focused(cx, window, more.clone()) {
            break;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
    }
    assert!(focused(cx, window, more.clone()));
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    assert!(focused(cx, window, format!("favorite-queue:{url}")));
    act(cx, window, |window, cx| window.press("escape", cx));
    assert!(focused(cx, window, more));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn nonsearch_save_keeps_its_captured_url_when_the_source_row_disappears(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 1, 0));
    let url = queue_item(10).url;
    click(cx, window, ("queue-favorite", 10_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::SetFavorite { id: 1, url: sent, saved: true }) if sent == url
    ));
    let mut source_gone = with_lists(Phase::Ready, 0, 0);
    source_gone.favorites = vec![FavoriteEntry {
        url: url.clone(),
        title: Some("Queued 10".into()),
        ..favorite("bbbbbbbbbbb", None)
    }];
    states.send_replace(ack(source_gone, 1, &url, true));
    cx.run_until_parked();
    cx.update(|cx| assert!(view.read(cx).favorite_pending.is_none()));
    show(cx, &view, Section::Favorites);
    assert!(present(cx, window, format!("favorite-row:{url}")));
}

#[gpui_kit::test]
fn settings_and_search_exits_preserve_the_selected_collection(cx: &mut TestAppContext) {
    let (window, view, mut received, _) = open(cx, state(Phase::Ready));
    act(cx, window, |window, cx| window.press("ctrl-3", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Favorites));
    click(cx, window, "settings");
    assert!(cx.update(|cx| view.read(cx).settings_open));
    act(cx, window, |window, cx| window.press("escape", cx));
    cx.update(|cx| assert!(!view.read(cx).settings_open));
    assert!(focused(cx, window, "url-editor"));
    submit_search(cx, window, &mut received);
    assert!(present(cx, window, "search-region"));
    click(cx, window, "search-cancel");
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Favorites));
    assert!(present(cx, window, "favorites-empty"));
}

#[gpui_kit::test]
fn ctrl_digits_and_tabs_select_all_three_collections(cx: &mut TestAppContext) {
    let (window, view, _, _) = open(cx, with_lists(Phase::Ready, 1, 1));
    act(cx, window, |window, cx| window.press("ctrl-3", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Favorites));
    assert!(present(cx, window, "favorites-empty"));
    act(cx, window, |window, cx| window.press("ctrl-2", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::History));
    act(cx, window, |window, cx| window.press("ctrl-1", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
}

#[gpui_kit::test]
fn narrow_window_uses_the_chooser_and_wide_uses_tabs(cx: &mut TestAppContext) {
    let narrow = with_lists(Phase::Ready, 1, 1);
    let ((window, _, _, _), _) = open_at(cx, narrow.clone(), 420., 480.);
    assert!(present(cx, window, "collection-selector"));
    let (window, _, _, _) = open(cx, narrow);
    assert!(!present(cx, window, "collection-selector"));
    act(cx, window, |window, cx| {
        window.within("sections").click(2_usize, cx);
    });
    assert!(present(cx, window, "favorites-empty"));
}

#[gpui_kit::test]
fn count_only_updates_do_not_navigate_or_emit_selection(cx: &mut TestAppContext) {
    let (_window, view, mut received, states) = open(
        cx,
        with_favorites(Phase::Ready, vec![favorite("aaaaaaaaaaa", None)]),
    );
    show(cx, &view, Section::Favorites);
    let mut count_changed = with_favorites(
        Phase::Ready,
        vec![favorite("aaaaaaaaaaa", None), favorite("bbbbbbbbbbb", None)],
    );
    count_changed.queue = with_lists(Phase::Ready, 3, 0).queue;
    states.send_replace(count_changed);
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Favorites));
    assert!(
        received.try_recv().is_err(),
        "count changes emit no command"
    );
}

#[gpui_kit::test]
fn empty_favorites_explains_itself(cx: &mut TestAppContext) {
    let (window, view, _, _) = open(cx, state(Phase::Ready));
    show(cx, &view, Section::Favorites);
    assert!(present(cx, window, "favorites-empty"));
    let text = cx
        .update_window(window, |_, window, _| {
            window
                .find("favorites-empty")
                .label()
                .unwrap_or_default()
                .to_owned()
        })
        .ok();
    assert_eq!(
        text.as_deref(),
        Some("Keep discoveries here. Save a track from playback, Up next, History or Find Music.")
    );
}

#[gpui_kit::test]
fn history_clear_confirmation_is_preserved_at_both_sizes(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, view, mut received, _), _) =
            open_at(cx, with_lists(Phase::Ready, 0, 2), width, height);
        show(cx, &view, Section::History);
        assert_eq!(button(cx, window, "history-clear").0, "Clear history");
        click(cx, window, "history-clear");
        assert!(received.try_recv().is_err(), "first click asks");
        assert_eq!(button(cx, window, "history-clear").0, "Yes, clear");
        assert!(present(cx, window, "history-clear-cancel"));
        click(cx, window, "history-clear-cancel");
        assert_eq!(button(cx, window, "history-clear").0, "Clear history");
    }
}
