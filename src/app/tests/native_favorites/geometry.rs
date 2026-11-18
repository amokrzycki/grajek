use super::*;

fn ids(state: &State) -> Vec<ElementId> {
    let mut ids: Vec<ElementId> = [
        "player-header",
        "active-player",
        "queue-region",
        "sections",
        "collection-selector",
        "now-playing-favorite",
        "now-playing-saved",
        "search-favorite",
        "search-saved",
        "search-region",
        "search-list",
        "search-actions",
        "link-entry",
        "url",
        "audio-footer",
        "queue-list",
        "history-list",
        "favorites-list",
        "history-clear",
        "history-clear-cancel",
        "track-options-menu",
        "help-popup",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    for item in state.favorites.iter().take(3) {
        for prefix in [
            "favorite-row",
            "favorite-play",
            "favorite-title",
            "favorite-more",
            "favorite-queue",
            "favorite-next",
            "favorite-remove",
        ] {
            ids.push(format!("{prefix}:{}", item.url).into());
        }
    }
    for item in state.queue.iter().take(3) {
        for prefix in [
            "queue-row",
            "queue-play",
            "queue-title",
            "queue-more",
            "queue-next",
            "queue-up",
            "queue-down",
            "queue-remove",
            "queue-favorite",
        ] {
            ids.push((prefix, item.id).into());
        }
    }
    for index in 0_usize..3.min(state.history.len()) {
        for prefix in [
            "history-row",
            "history-play",
            "history-title",
            "history-more",
            "history-queue",
            "history-next",
            "history-remove",
            "history-favorite",
        ] {
            ids.push((prefix, index).into());
        }
    }
    ids
}

pub(super) fn capture(
    window: &gpui_kit::Window,
    state: &State,
) -> serde_json::Map<String, serde_json::Value> {
    let mut result = serde_json::Map::new();
    result.insert("viewport".into(), serde_json::json!({"width": window.viewport_size().width.as_f32(), "height": window.viewport_size().height.as_f32(), "scale": window.scale_factor()}));
    for id in ids(state) {
        if let Some(element) = window.try_find(id.clone()) {
            let b = element.bounds();
            result.insert(format!("{id:?}"), serde_json::json!({"x": b.left().as_f32(), "y": b.top().as_f32(), "width": b.size.width.as_f32(), "height": b.size.height.as_f32(), "label": element.label(), "disabled": element.disabled()}));
        }
    }
    result.insert(
        "registered-paths".into(),
        serde_json::json!(gpui_kit::base::test_support::registered_paths(window)),
    );
    let observed: Vec<_> = gpui_kit::base::test_support::snapshots(window).iter().map(|element| {
        let b = element.bounds();
        serde_json::json!({"path": format!("{:?}", element.path()), "label": element.label(), "value": element.value(), "expanded": element.expanded(), "selected": element.selected(), "x": b.left().as_f32(), "y": b.top().as_f32(), "width": b.size.width.as_f32(), "height": b.size.height.as_f32()})
    }).collect();
    result.insert("observed-elements".into(), serde_json::json!(observed));
    result
}

pub(super) fn assert_bounds(name: &str, window: &gpui_kit::Window, state: &State, two_rows: bool) {
    let viewport = Bounds {
        origin: point(px(0.), px(0.)),
        size: window.viewport_size(),
    };
    for id in ids(state) {
        if let Some(element) = window.try_find(id.clone()) {
            let b = element.bounds();
            let key = format!("{id:?}");
            if ["-row", "-title", "-play", "-more"]
                .iter()
                .any(|part| key.contains(part))
                && !key.contains("active-player")
            {
                assert!(
                    b.left() >= px(0.) && b.right() <= viewport.right() + px(1.),
                    "{name}: row/action horizontal clipping: {id:?} {b:?}"
                );
                continue;
            }
            assert!(
                b.left() >= px(0.)
                    && b.right() <= viewport.right() + px(1.)
                    && b.top() >= px(0.)
                    && b.bottom() <= viewport.bottom() + px(1.),
                "{name}: {id:?} clipped: {b:?}, viewport {viewport:?}"
            );
        }
    }
    for element in gpui_kit::base::test_support::snapshots(window) {
        if element.path().contains(&ElementId::from("sections")) {
            let b = element.bounds();
            assert!(
                b.left() >= px(0.)
                    && b.right() <= viewport.right()
                    && b.top() >= px(0.)
                    && b.bottom() <= viewport.bottom(),
                "{name}: tab clipped: {:?} {b:?}",
                element.path()
            );
            if let Some(clear) = window.try_find("history-clear") {
                assert!(
                    b.right() <= clear.bounds().left(),
                    "{name}: collection tabs overlap Clear history: {b:?}"
                );
            }
        }
    }
    let footer = window.find("audio-footer").bounds();
    for list_id in ["queue-list", "history-list", "favorites-list"] {
        if let Some(list) = window.try_find(list_id) {
            let list = list.bounds();
            assert!(
                list.bottom() <= footer.top(),
                "{name}: list overlaps footer"
            );
            if two_rows {
                assert!(
                    list.size.height >= px(88.),
                    "{name}: two full 44px rows required: {list:?}"
                );
                let rows: Vec<ElementId> = match list_id {
                    "queue-list" => state
                        .queue
                        .iter()
                        .take(2)
                        .map(|item| ("queue-row", item.id).into())
                        .collect(),
                    "history-list" => (0_usize..2)
                        .map(|index| ("history-row", index).into())
                        .collect(),
                    _ => state
                        .favorites
                        .iter()
                        .take(2)
                        .map(|item| format!("favorite-row:{}", item.url).into())
                        .collect(),
                };
                for id in rows {
                    let row = window.find(id.clone()).bounds();
                    assert_eq!(row.size.height, px(44.), "{name}: {id:?} shared row height");
                    assert!(
                        row.top() >= list.top()
                            && row.bottom() <= list.bottom()
                            && row.left() >= list.left()
                            && row.right() <= list.right(),
                        "{name}: two complete visible rows: {row:?} outside {list:?}"
                    );
                }
            }
        }
    }
    assert_menu_bounds(name, window, state);
}

fn assert_menu_bounds(name: &str, window: &gpui_kit::Window, state: &State) {
    if let Some(menu) = window.try_find("track-options-menu") {
        let menu = menu.bounds();
        for item in state.favorites.iter().take(3) {
            for prefix in ["favorite-queue", "favorite-next", "favorite-remove"] {
                if let Some(action) = window.try_find(format!("{prefix}:{}", item.url)) {
                    let b = action.bounds();
                    assert!(
                        b.left() >= menu.left()
                            && b.right() <= menu.right()
                            && b.top() >= menu.top()
                            && b.bottom() <= menu.bottom(),
                        "{name}: menu action clipped {b:?} outside {menu:?}"
                    );
                }
            }
        }
    }
}
