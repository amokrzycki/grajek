use super::*;
use crate::player::{SearchPhase, SearchState};

pub(super) const CASES: [&str; 9] = [
    "search-loading",
    "search-results",
    "search-missing",
    "search-empty",
    "search-error",
    "search-selected",
    "search-import-stress",
    "search-keyboard",
    "search-queue-next",
];

pub(super) fn snapshot(name: &str, mut state: State, id: u64) -> State {
    let phase = match name {
        "search-loading" => SearchPhase::Loading,
        "search-empty" => SearchPhase::Empty { rejected: 0 },
        "search-error" => SearchPhase::Failed(crate::search::Error::Unavailable),
        "search-missing" => {
            let SearchPhase::Results(mut results) = super::search::results() else {
                unreachable!()
            };
            results.items[0].title = None;
            results.items[0].channel = None;
            results.items[0].duration = None;
            results.items[1].live = true;
            results.items[1].duration = None;
            SearchPhase::Results(results)
        }
        _ => super::search::results(),
    };
    if name == "search-import-stress" {
        state.import = crate::player::ImportStatus::Importing;
        state.error = Some("Playback interrupted. Check the connection and try again.".into());
    }
    state.search = SearchState {
        id,
        query: "Bach cello".into(),
        phase,
        status: None,
    };
    state
}

pub(super) fn assert_geometry(name: &str, window: &gpui_kit::Window) {
    let list = window.find("search-list").bounds();
    assert!(list.size.height >= px(44.), "{name}: {list:?}");
    assert!(list.bottom() <= window.find("audio-footer").bounds().top());
    if !matches!(name, "search-loading" | "search-empty" | "search-error") {
        let row = window
            .find(if name == "search-keyboard" {
                "search-row-4"
            } else {
                "search-row-0"
            })
            .bounds();
        assert!(
            row.top() >= list.top() && row.bottom() <= list.bottom(),
            "{name}: {row:?}, {list:?}"
        );
        let actions = window.find("search-actions").bounds();
        assert!(actions.bottom() <= window.find("audio-footer").bounds().top());
        for id in ["search-play", "search-enqueue", "search-enqueue-next"] {
            let button = window.find(id).bounds();
            assert!(
                button.left() >= actions.left()
                    && button.right() <= actions.right()
                    && button.top() >= actions.top()
                    && button.bottom() <= actions.bottom(),
                "{name}: {id} {button:?} outside {actions:?}"
            );
        }
    }
}
