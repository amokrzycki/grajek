mod geometry;
mod navigation;

use super::*;
use crate::player::{EnqueueResult, FavoriteRefusal, FavoriteResult, Placement, SearchAction};
use crate::store::FavoriteEntry;
use gpui_kit::{AsyncApp, BackgroundExecutor};
use std::path::Path;

pub(super) const CASES: [&str; 22] = [
    "fav-playing",
    "fav-paused",
    "fav-connecting",
    "fav-resolving",
    "fav-queue",
    "fav-history",
    "fav-search",
    "fav-play",
    "fav-end",
    "fav-next",
    "fav-disconnected",
    "fav-full-queue",
    "fav-capacity",
    "fav-long",
    "fav-missing",
    "fav-focus",
    "fav-focus-deep",
    "fav-chooser",
    "fav-history-clear",
    "fav-maxcounts",
    "fav-resize",
    "fav-breakpoint",
];
const CURRENT: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const FULL: &str = "Favorites full (500 tracks). Remove a favorite and try again.";

fn entry(url: &str, title: Option<&str>) -> FavoriteEntry {
    FavoriteEntry {
        url: url.into(),
        title: title.map(str::to_owned),
        channel: None,
        duration_secs: Some(225),
        live: false,
    }
}

pub(super) fn snapshot(name: &str, mut base: State) -> State {
    base.phase = match name {
        "fav-paused" => Phase::Paused,
        "fav-connecting" => Phase::Connecting,
        "fav-resolving" => Phase::Resolving,
        _ => Phase::Playing,
    };
    base.current_url = Some(CURRENT.into());
    base.favorites = vec![
        entry(
            "https://www.youtube.com/watch?v=aaaaaaaaaaa",
            Some("Ólafur Arnalds — Near Light"),
        ),
        entry(
            "https://www.youtube.com/watch?v=bbbbbbbbbbb",
            Some("Hania Rani — Glass"),
        ),
        entry(
            "https://www.youtube.com/watch?v=ccccccccccc",
            Some("坂本龍一 — Merry Christmas Mr. Lawrence"),
        ),
    ];
    if matches!(
        name,
        "fav-capacity"
            | "fav-maxcounts"
            | "fav-history-clear"
            | "fav-chooser"
            | "fav-resize"
            | "fav-breakpoint"
    ) {
        base.queue = with_lists(base.phase, 100, 50).queue;
        base.history = with_lists(base.phase, 100, 50).history;
        base.favorites = (1000..1500)
            .map(|id| {
                entry(
                    &format!("https://www.youtube.com/watch?v={id:011}"),
                    Some(&format!("Saved {id} · Żółć · 音楽")),
                )
            })
            .collect();
    }
    if name == "fav-full-queue" {
        base.queue = with_lists(base.phase, 100, 0).queue;
    }
    if name == "fav-focus-deep" {
        base.favorites = (2000..2020)
            .map(|id| {
                entry(
                    &format!("https://www.youtube.com/watch?v={id:011}"),
                    Some("Saved track"),
                )
            })
            .collect();
    }
    if name == "fav-disconnected" {
        base.available = false;
        base.voice_connected = false;
    }
    if name == "fav-long" {
        base.favorites[0].title = Some("Long favorite · Żółć · 音楽 · Привет ".repeat(5));
    }
    if name == "fav-missing" {
        base.favorites[0].title = None;
        base.favorites[0].duration_secs = None;
        base.favorites[0].channel = None;
    }
    base
}

pub(super) struct Fixture<'a> {
    pub view: &'a Entity<PlayerApp>,
    pub window: AnyWindowHandle,
    pub executor: &'a BackgroundExecutor,
    pub snapshots: &'a watch::Sender<State>,
    pub receiver: &'a mut mpsc::Receiver<Command>,
    pub directory: &'a Path,
    pub width: f32,
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "opt-in native assertions fail on window or command errors"
)]
impl Fixture<'_> {
    fn ui<R>(
        &self,
        cx: &mut AsyncApp,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        cx.update_window(self.window, |_, window, cx| f(window, cx))
            .expect("native Favorites window")
    }

    async fn settle(&self) {
        self.executor.timer(Duration::from_millis(250)).await;
    }

    async fn publish(&self, state: &State) {
        self.snapshots.send_replace(state.clone());
        self.settle().await;
    }

    fn click(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) {
        let id = id.into();
        self.ui(cx, |window, cx| {
            window.click(id, cx);
            window.render_frame(cx);
        });
    }

    fn press(&self, cx: &mut AsyncApp, key: &str) {
        self.ui(cx, |window, cx| {
            window.press(key, cx);
            window.render_frame(cx);
        });
    }

    fn tab_to(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) {
        let id = id.into();
        self.ui(cx, |window, cx| {
            window.focus(&self.view.read(cx).input.read(cx).focus_handle(cx), cx);
            for _ in 0..80 {
                window.render_frame(cx);
                let focused = if id == ElementId::from("collection-selector") {
                    self.view
                        .read(cx)
                        .collection_select
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                } else {
                    window.find(id.clone()).focused() == Some(true)
                };
                if focused {
                    return;
                }
                window.press(
                    if id == ElementId::from("now-playing-favorite") {
                        "shift-tab"
                    } else {
                        "tab"
                    },
                    cx,
                );
            }
            assert_eq!(
                window.find(id).focused(),
                Some(true),
                "Tab reaches Favorites control"
            );
        });
    }

    fn focused(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.ui(cx, |window, _| window.find(id).focused() == Some(true))
    }

    fn label(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) -> String {
        let id = id.into();
        self.ui(cx, |window, _| {
            window.find(id).label().unwrap_or_default().to_owned()
        })
    }

    fn present(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.ui(cx, |window, _| window.try_find(id).is_some())
    }

    fn disabled(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.ui(cx, |window, _| {
            let element = window.find(id);
            element
                .disabled()
                .unwrap_or_else(|| element.focused().is_none())
        })
    }

    fn section(&self, cx: &mut AsyncApp, section: Section) {
        self.ui(cx, |window, cx| {
            window.focus(&self.view.read(cx).input.read(cx).focus_handle(cx), cx);
        });
        self.press(
            cx,
            match section {
                Section::Queue => "ctrl-1",
                Section::History => "ctrl-2",
                Section::Favorites => "ctrl-3",
            },
        );
        self.ui(cx, |_, cx| assert_eq!(self.view.read(cx).section, section));
    }

    fn no_commands(&mut self) {
        assert!(
            self.receiver.try_recv().is_err(),
            "no extra command, including CancelSearch"
        );
    }

    fn membership(&mut self, url: &str, saved: bool) -> u64 {
        let Command::SetFavorite {
            id,
            url: actual,
            saved: desired,
        } = self.receiver.try_recv().expect("membership command")
        else {
            panic!("expected raw SetFavorite");
        };
        assert_eq!(actual, url);
        assert_eq!(desired, saved);
        self.no_commands();
        id
    }

    fn answer(state: &mut State, request_id: u64, url: &str, saved: bool) {
        if saved && !state.is_favorite(url) {
            state
                .favorites
                .insert(0, entry(url, Some("Confirmed saved track")));
        }
        if !saved {
            state.favorites.retain(|item| item.url != url);
        }
        state.favorite_result = Some(FavoriteResult {
            request_id,
            url: url.into(),
            saved,
            result: Ok(()),
        });
    }

    fn record(&self, name: &str, text: &str) {
        std::fs::write(self.directory.join(format!("{name}-{:.0}.interaction.txt", self.width)),
            format!("{text}\nOffline scripted snapshots and native input; no controller persistence, network, Discord or audio.\n")).expect("Favorites interaction evidence");
    }

    fn geometry(&self, name: &str, cx: &mut AsyncApp) {
        self.ui(cx, |window, cx| {
            let geometry = geometry::capture(window, &self.view.read(cx).state);
            std::fs::write(
                self.directory
                    .join(format!("{name}-{:.0}.favorites.geometry.json", self.width)),
                serde_json::to_vec_pretty(&geometry).expect("Favorites geometry JSON"),
            )
            .expect("Favorites geometry evidence");
        });
    }

    #[expect(clippy::future_not_send, reason = "native foreground capture")]
    async fn checkpoint(&self, name: &str, cx: &mut AsyncApp) {
        self.settle().await;
        self.geometry(name, cx);
        #[cfg(target_os = "linux")]
        let (width, height) = self.ui(cx, |window, _| {
            (
                window.viewport_size().width.as_f32(),
                window.viewport_size().height.as_f32(),
            )
        });
        let path = self.directory.join(format!("{name}-{:.0}.png", self.width));
        #[cfg(target_os = "linux")]
        {
            let mut capture = std::process::Command::new("python3")
                .arg("scripts/capture-native-window.py")
                .arg(std::process::id().to_string())
                .arg(&path)
                .arg(format!("{width:.0}"))
                .arg(format!("{height:.0}"))
                .spawn()
                .expect("capture Favorites checkpoint");
            let mut status = None;
            for _ in 0..100 {
                status = capture.try_wait().expect("Favorites checkpoint status");
                if status.is_some() {
                    break;
                }
                self.executor.timer(Duration::from_millis(50)).await;
            }
            if status.is_none() {
                capture.kill().expect("stop stalled checkpoint capture");
            }
            assert!(
                status.is_some_and(|status| status.success()),
                "Favorites checkpoint capture failed"
            );
        }
        #[cfg(not(target_os = "linux"))]
        self.ui(cx, |window, _| {
            window
                .render_to_image()
                .expect("checkpoint pixels")
                .save(path)
                .expect("save checkpoint");
        });
    }

    fn bounds(&self, name: &str, cx: &mut AsyncApp, two_rows: bool) {
        self.geometry(name, cx);
        self.ui(cx, |window, cx| {
            geometry::assert_bounds(name, window, &self.view.read(cx).state, two_rows);
        });
    }

    #[expect(
        clippy::future_not_send,
        clippy::too_many_lines,
        reason = "native foreground scripts preserve exact input and acknowledgement order"
    )]
    pub(super) async fn drive(&mut self, name: &str, base: &State, cx: &mut AsyncApp) -> usize {
        let mut state = base.clone();
        self.record(
            name,
            "Started; assertions after this checkpoint must pass before this case is accepted.",
        );
        match name {
            "fav-playing" | "fav-paused" | "fav-connecting" | "fav-resolving"
            | "fav-disconnected" => {
                assert_eq!(state.current_url.as_deref(), Some(CURRENT));
                let before_track = state.track.clone();
                let before_queue = state.queue.clone();
                self.toggle(
                    name,
                    &mut state,
                    "now-playing-favorite".into(),
                    "now-playing-saved".into(),
                    CURRENT,
                    cx,
                )
                .await;
                self.ui(cx, |_, cx| {
                    let app = self.view.read(cx);
                    assert_eq!(app.state.current_url.as_deref(), Some(CURRENT));
                    assert_eq!(app.state.track, before_track);
                    assert_eq!(app.state.queue, before_queue);
                    assert_eq!(app.state.phase, base.phase);
                });
                self.record(name, "Save then Remove: exact current_url, one raw SetFavorite each, labels and separate Saved wait for matching acknowledgements; track, phase and queue retained.");
            }
            "fav-queue" | "fav-history" => {
                let history = name == "fav-history";
                self.section(
                    cx,
                    if history {
                        Section::History
                    } else {
                        Section::Queue
                    },
                );
                let url = if history {
                    state.history[0].url.clone()
                } else {
                    state.queue[0].url.clone()
                };
                let trigger: ElementId = if history {
                    ("history-more", 0_usize).into()
                } else {
                    ("queue-more", state.queue[0].id).into()
                };
                let action: ElementId = if history {
                    ("history-favorite", 0_usize).into()
                } else {
                    ("queue-favorite", state.queue[0].id).into()
                };
                let saved: ElementId = if history {
                    ("history-saved", 0_usize).into()
                } else {
                    ("queue-saved", state.queue[0].id).into()
                };
                for desired in [true, false] {
                    self.click(cx, trigger.clone());
                    for id in if history {
                        vec![
                            ElementId::from(("history-queue", 0_usize)),
                            ("history-next", 0_usize).into(),
                            ("history-remove", 0_usize).into(),
                        ]
                    } else {
                        vec![
                            ("queue-next", state.queue[0].id).into(),
                            ("queue-up", state.queue[0].id).into(),
                            ("queue-down", state.queue[0].id).into(),
                            ("queue-remove", state.queue[0].id).into(),
                        ]
                    } {
                        assert!(
                            self.present(cx, id),
                            "membership retains existing menu actions"
                        );
                    }
                    self.bounds(name, cx, false);
                    assert_eq!(
                        self.label(cx, action.clone()),
                        if desired {
                            "Save to Favorites"
                        } else {
                            "Remove from Favorites"
                        }
                    );
                    self.click(cx, action.clone());
                    let id = self.membership(&url, desired);
                    assert_eq!(
                        self.present(cx, saved.clone()),
                        !desired,
                        "no optimistic row membership"
                    );
                    Self::answer(&mut state, id, &url, desired);
                    self.publish(&state).await;
                    assert_eq!(self.present(cx, saved.clone()), desired);
                }
                self.record(name, "Existing More actions and geometry retained; Save/Remove use captured row URL; Saved indicator changes only on matching scripted acknowledgement.");
            }
            "fav-search" => {
                self.search(name, &mut state, cx).await;
            }
            "fav-play" | "fav-end" | "fav-next" => {
                self.section(cx, Section::Favorites);
                let url = state.favorites[0].url.clone();
                self.bounds(name, cx, true);
                if name == "fav-play" {
                    self.tab_to(cx, format!("favorite-play:{url}"));
                    self.press(cx, "enter");
                    let Command::Transport { id, command } =
                        self.receiver.try_recv().expect("Play transport")
                    else {
                        panic!("expected raw transport envelope");
                    };
                    assert!(matches!(*command, Command::Play(actual) if actual == url));
                    self.no_commands();
                    self.ui(cx, |_, cx| {
                        assert_eq!(self.view.read(cx).state.current_url, base.current_url);
                    });
                    state.transport_ack = id;
                    state.current_url = Some(url);
                } else {
                    self.click(cx, format!("favorite-more:{url}"));
                    self.bounds(name, cx, false);
                    self.click(
                        cx,
                        format!(
                            "favorite-{}:{url}",
                            if name == "fav-end" { "queue" } else { "next" }
                        ),
                    );
                    let Command::EnqueueRequest {
                        id,
                        value,
                        placement,
                    } = self.receiver.try_recv().expect("enqueue request")
                    else {
                        panic!("expected raw EnqueueRequest for both placements");
                    };
                    assert_eq!(value, url);
                    assert_eq!(
                        placement,
                        if name == "fav-end" {
                            Placement::End
                        } else {
                            Placement::Next
                        }
                    );
                    self.no_commands();
                    self.ui(cx, |_, cx| {
                        assert_eq!(self.view.read(cx).state.queue, base.queue);
                        assert!(self.view.read(cx).enqueue_pending.is_some());
                    });
                    let item = QueueItem {
                        id: 900,
                        url,
                        title: Some("Explicit insertion".into()),
                        title_unavailable: false,
                    };
                    if placement == Placement::End {
                        state.queue.push(item);
                    } else {
                        state.queue.insert(0, item);
                    }
                    state.enqueue_result = Some(EnqueueResult {
                        request_id: id,
                        item_id: Some(900),
                    });
                }
                self.publish(&state).await;
                self.ui(cx, |_, cx| {
                    assert_eq!(self.view.read(cx).state.queue, state.queue);
                    assert_eq!(self.view.read(cx).state.current_url, state.current_url);
                    assert!(self.view.read(cx).enqueue_pending.is_none());
                });
                self.record(name, "Explicit Favorite action: exact raw Play transport or EnqueueRequest with End/Next placement; pending retains confirmed snapshot, matching acknowledgement applies result.");
            }
            "fav-full-queue" => {
                self.toggle(
                    name,
                    &mut state,
                    "now-playing-favorite".into(),
                    "now-playing-saved".into(),
                    CURRENT,
                    cx,
                )
                .await;
                self.section(cx, Section::Favorites);
                let url = state.favorites[0].url.clone();
                self.click(cx, format!("favorite-more:{url}"));
                assert!(self.disabled(cx, format!("favorite-next:{url}")));
                assert!(!self.disabled(cx, format!("favorite-queue:{url}")));
                assert!(!self.disabled(cx, format!("favorite-remove:{url}")));
                self.bounds(name, cx, false);
                self.click(cx, format!("favorite-queue:{url}"));
                let Command::EnqueueRequest {
                    id,
                    value,
                    placement: Placement::End,
                } = self.receiver.try_recv().expect("full Add request")
                else {
                    panic!("full queue Add remains requestable");
                };
                assert_eq!(value, url);
                self.no_commands();
                state.enqueue_result = Some(EnqueueResult {
                    request_id: id,
                    item_id: None,
                });
                self.publish(&state).await;
                self.click(cx, format!("favorite-more:{url}"));
                self.click(cx, format!("favorite-remove:{url}"));
                let id = self.membership(&url, false);
                Self::answer(&mut state, id, &url, false);
                self.publish(&state).await;
                assert_eq!(state.queue.len(), 100);
                self.record(name, "Queue full: Next disabled, Add sends End and receives scripted refusal; Remove remains requestable and confirmed without queue mutation.");
            }
            "fav-capacity" => {
                self.click(cx, "now-playing-favorite");
                let id = self.membership(CURRENT, true);
                state.favorite_result = Some(FavoriteResult {
                    request_id: id,
                    url: CURRENT.into(),
                    saved: true,
                    result: Err(FavoriteRefusal::Full),
                });
                self.publish(&state).await;
                assert_eq!(self.label(cx, "link-status"), FULL);
                assert!(!self.present(cx, "now-playing-saved"));
                self.ui(cx, |_, cx| {
                    assert_eq!(self.view.read(cx).state.favorites, base.favorites);
                    assert_eq!(self.view.read(cx).state.phase, base.phase);
                });
                self.section(cx, Section::Favorites);
                self.bounds(name, cx, true);
                self.record(name, "501st unique Save: raw exact-URL request, scripted Full refusal; exact recovery text; 500 entries retained, no Saved claim, playback phase retained.");
            }
            "fav-long" | "fav-missing" => {
                self.metadata(name, &state, cx).await;
            }
            "fav-focus" => {
                self.focus(name, &mut state, cx).await;
            }
            "fav-focus-deep" => self.deep_focus(name, &mut state, cx).await,
            "fav-chooser" => {
                self.chooser(name, &mut state, cx).await;
            }
            "fav-history-clear" => {
                self.section(cx, Section::History);
                self.click(cx, "history-clear");
                self.no_commands();
                assert_eq!(self.label(cx, "history-clear"), "Yes, clear");
                assert!(self.present(cx, "history-clear-cancel"));
                self.bounds(&format!("{name}-confirmation"), cx, true);
                self.checkpoint(&format!("{name}-confirmation"), cx).await;
                self.click(cx, "history-clear-cancel");
                assert_eq!(self.label(cx, "history-clear"), "Clear history");
                self.no_commands();
                self.checkpoint(&format!("{name}-cancelled"), cx).await;
                self.click(cx, "history-clear");
                self.bounds(&format!("{name}-reconfirmation"), cx, true);
                self.click(cx, "history-clear");
                assert!(matches!(
                    self.receiver.try_recv(),
                    Ok(Command::ClearHistory)
                ));
                self.no_commands();
                self.ui(cx, |_, cx| {
                    assert_eq!(self.view.read(cx).state.history.len(), 50);
                });
                state.history.clear();
                self.publish(&state).await;
                self.ui(cx, |_, cx| {
                    assert_eq!(self.view.read(cx).state.history, []);
                    assert_eq!(self.view.read(cx).state.favorites.len(), 500);
                    assert_eq!(self.view.read(cx).state.queue.len(), 100);
                });
                self.record(name, "100 queue/50 history/500 favorites: Clear opens confirmation, Cancel preserves; second explicit confirmation sends exactly ClearHistory, waits for snapshot, Favorites and queue retained. Confirmation geometry checked with two full rows.");
            }
            "fav-maxcounts" => {
                for section in [Section::Queue, Section::History, Section::Favorites] {
                    self.section(cx, section);
                    self.bounds(&format!("{name}-{section:?}"), cx, true);
                }
                let first = state.favorites[0].url.clone();
                self.tab_to(cx, format!("favorite-more:{first}"));
                self.press(cx, "end");
                self.settle().await;
                let last = state.favorites.last().expect("500 favorites").url.clone();
                assert!(self.focused(cx, format!("favorite-more:{last}")));
                self.visible(cx, &format!("favorite-row:{last}"), "favorites-list");
                self.press(cx, "home");
                self.settle().await;
                assert!(self.focused(cx, format!("favorite-more:{first}")));
                self.visible(cx, &format!("favorite-row:{first}"), "favorites-list");
                self.no_commands();
                self.record(name, "Ctrl+1/2/3 at 100/50/500; two full 44px rows and header/navigation/list/footer fit each collection; Home/End focus and reveal first/500th Favorite without playback commands.");
            }
            "fav-breakpoint" => self.breakpoint(name, cx).await,
            "fav-resize" => {
                self.resize(name, cx).await;
            }
            _ => panic!("unimplemented Favorites case {name}"),
        }
        self.bounds(name, cx, false);
        self.no_commands();
        state.queue.len()
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    async fn toggle(
        &mut self,
        name: &str,
        state: &mut State,
        control: ElementId,
        indicator: ElementId,
        url: &str,
        cx: &mut AsyncApp,
    ) {
        for saved in [true, false] {
            self.tab_to(cx, control.clone());
            assert!(self.label(cx, control.clone()).starts_with(if saved {
                "Save"
            } else {
                "Remove"
            }));
            assert_eq!(self.present(cx, indicator.clone()), !saved);
            self.press(cx, "enter");
            let id = self.membership(url, saved);
            self.press(cx, "enter");
            self.no_commands();
            assert!(
                self.label(cx, control.clone())
                    .starts_with(if saved { "Save" } else { "Remove" }),
                "pending label is confirmed state"
            );
            assert_eq!(self.present(cx, indicator.clone()), !saved);
            Self::answer(state, id, url, saved);
            self.publish(state).await;
            assert!(self.label(cx, control.clone()).starts_with(if saved {
                "Remove"
            } else {
                "Save"
            }));
            assert_eq!(self.present(cx, indicator.clone()), saved);
            assert!(
                self.focused(cx, control.clone()),
                "acknowledgement retains focus"
            );
            self.bounds(
                &format!("{name}-{}", if saved { "saved" } else { "removed" }),
                cx,
                false,
            );
        }
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    async fn search(&mut self, name: &str, state: &mut State, cx: &mut AsyncApp) {
        self.click(cx, "url");
        self.ui(cx, |window, cx| window.input("Bach cello", cx));
        self.press(cx, "enter");
        let Command::Search {
            id: search_id,
            query,
        } = self.receiver.try_recv().expect("search intent")
        else {
            panic!("expected Search");
        };
        assert_eq!(query, "Bach cello");
        self.no_commands();
        *state = super::native_search::snapshot("search-results", state.clone(), search_id);
        self.publish(state).await;
        assert!(self.disabled(cx, "search-favorite"));
        self.click(cx, "search-row-0");
        let url = "https://www.youtube.com/watch?v=aaaaaaaaaa0";
        for saved in [true, false] {
            self.tab_to(cx, "search-favorite");
            assert!(self.label(cx, "search-favorite").starts_with(if saved {
                "Save"
            } else {
                "Remove"
            }));
            self.press(cx, "enter");
            let Command::SearchAction {
                search_id: actual,
                video_id,
                action: SearchAction::Favorite { id, saved: desired },
            } = self.receiver.try_recv().expect("search Favorite")
            else {
                panic!("expected identity-bound SearchAction::Favorite");
            };
            assert_eq!(actual, search_id);
            assert_eq!(video_id, "aaaaaaaaaa0");
            assert_eq!(desired, saved);
            self.no_commands();
            assert_eq!(self.present(cx, "search-saved"), !saved);
            Self::answer(state, id, url, saved);
            self.publish(state).await;
            assert_eq!(self.present(cx, "search-saved"), saved);
            assert!(self.label(cx, "search-favorite").starts_with(if saved {
                "Remove"
            } else {
                "Save"
            }));
            assert!(self.focused(cx, "search-favorite"));
            self.ui(cx, |window, cx| {
                assert!(self.view.read(cx).searching());
                assert_eq!(
                    self.view.read(cx).input.read(cx).value().as_str(),
                    "Bach cello"
                );
                let crate::player::SearchPhase::Results(results) = &state.search.phase else {
                    panic!("Results retained");
                };
                assert!(
                    window
                        .find("search-play")
                        .label()
                        .unwrap_or_default()
                        .contains(results.items[0].title.as_deref().expect("selected title"))
                );
                super::native_search::assert_geometry("search-selected", window);
            });
            self.bounds(name, cx, false);
        }
        self.record(name, "Selected heading Save/Remove sends exact session/video commands; matching acknowledgements retain Results, selected action identity, input and focused heading control; no CancelSearch. Existing search row/action bounds passed.");
    }

    fn visible(&self, cx: &mut AsyncApp, row: &str, list: &str) {
        self.ui(cx, |window, _| {
            let row = window.find(row.to_owned()).bounds();
            let list = window.find(list.to_owned()).bounds();
            assert!(
                row.top() >= list.top() && row.bottom() <= list.bottom(),
                "focused row {row:?} outside {list:?}"
            );
        });
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    async fn focus(&mut self, name: &str, state: &mut State, cx: &mut AsyncApp) {
        self.section(cx, Section::Favorites);
        let url = state.favorites[1].url.clone();
        self.tab_to(cx, format!("favorite-more:{url}"));
        state.favorites.insert(
            0,
            entry(
                "https://www.youtube.com/watch?v=ddddddddddd",
                Some("Inserted first"),
            ),
        );
        self.publish(state).await;
        assert!(
            self.focused(cx, format!("favorite-more:{url}")),
            "insertion retains same URL focus"
        );
        self.visible(cx, &format!("favorite-row:{url}"), "favorites-list");
        self.press(cx, "enter");
        self.settle().await;
        assert!(self.focused(cx, format!("favorite-queue:{url}")));
        self.bounds(name, cx, false);
        self.press(cx, "escape");
        assert!(self.focused(cx, format!("favorite-more:{url}")));
        self.click(cx, format!("favorite-more:{url}"));
        self.click(cx, format!("favorite-remove:{url}"));
        let id = self.membership(&url, false);
        Self::answer(state, id, &url, false);
        self.publish(state).await;
        let neighbor = state.favorites[2].url.clone();
        assert!(
            self.focused(cx, format!("favorite-more:{neighbor}")),
            "removed row falls forward to neighbour More"
        );
        assert!(!self.present(cx, "track-options-menu"));
        while !state.favorites.is_empty() {
            let url = state.favorites[0].url.clone();
            self.tab_to(cx, format!("favorite-more:{url}"));
            self.click(cx, format!("favorite-more:{url}"));
            self.click(cx, format!("favorite-remove:{url}"));
            let id = self.membership(&url, false);
            Self::answer(state, id, &url, false);
            self.publish(state).await;
        }
        self.ui(cx, |window, cx| {
            assert!(
                self.view
                    .read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
        assert!(self.present(cx, "favorites-empty"));
        self.record(name, "URL focus survives insertion and is visible; keyboard More focuses first action, Escape returns to trigger; explicit removal closes menu and focuses neighbour More; Explicit menu Remove removes remaining entries, last acknowledgement focuses composer.");
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    async fn metadata(&self, name: &str, state: &State, cx: &mut AsyncApp) {
        self.section(cx, Section::Favorites);
        let item = &state.favorites[0];
        let title = self.label(cx, format!("favorite-title:{}", item.url));
        assert!(!title.contains("Loading"));
        assert_eq!(
            title,
            item.title.clone().unwrap_or_else(|| "aaaaaaaaaaa".into())
        );
        self.bounds(name, cx, true);
        self.ui(cx, |window, cx| {
            window.hover(format!("favorite-title:{}", item.url), cx);
        });
        self.executor.timer(Duration::from_millis(800)).await;
        assert_eq!(self.label(cx, "help-popup"), title);
        self.bounds(&format!("{name}-tooltip"), cx, false);
        self.tab_to(cx, format!("favorite-more:{}", item.url));
        self.press(cx, "f1");
        self.settle().await;
        let help = self.label(cx, "help-popup");
        self.bounds(&format!("{name}-f1"), cx, false);
        self.record(name, &format!("Full title accessible and tooltip truthful: {title}\nMore F1 help: {help}\nURL requiring accessible help: {}", item.url));
    }

    #[expect(clippy::future_not_send, reason = "native foreground script")]
    async fn chooser(&mut self, name: &str, state: &mut State, cx: &mut AsyncApp) {
        self.section(cx, Section::Queue);
        if self.width > 480. {
            self.ui(cx, |window, cx| {
                window.within("sections").click(2_usize, cx);
                window.render_frame(cx);
            });
            self.ui(cx, |_, cx| {
                assert_eq!(self.view.read(cx).section, Section::Favorites);
            });
            self.bounds(name, cx, true);
            self.ui(cx, |window, _| window.resize(size(px(420.), px(480.))));
            self.settle().await;
            self.section(cx, Section::Queue);
        }
        self.tab_to(cx, "collection-selector");
        self.press(cx, "enter");
        self.press(cx, "down");
        self.press(cx, "down");
        self.checkpoint(&format!("{name}-open"), cx).await;
        self.press(cx, "enter");
        self.settle().await;
        self.ui(cx, |_, cx| {
            assert_eq!(
                self.view.read(cx).section,
                Section::Favorites,
                "native chooser confirms Favorites"
            );
        });
        assert!(self.present(cx, "favorites-list"));
        self.no_commands();
        state.queue.pop();
        self.publish(state).await;
        self.ui(cx, |_, cx| {
            assert_eq!(
                self.view.read(cx).section,
                Section::Favorites,
                "count-only change retains confirmed section"
            );
        });
        self.no_commands();
        self.bounds(name, cx, true);
        if self.width > 480. {
            self.ui(cx, |window, _| window.resize(size(px(520.), px(560.))));
            self.settle().await;
        }
        self.record(name, "Wide native tab selects Favorites; collection Select specifically reached by Tab at narrow size, Enter/Down/Down/Enter confirms Favorites; count-only snapshot retains selection and emits no commands. 100/50/500 stress.");
    }
}

pub(super) fn assert_after_capture(name: &str, window: &gpui_kit::Window, state: &State) {
    if matches!(name, "fav-long" | "fav-missing") {
        let help = window.find("help-popup");
        assert!(
            help.label()
                .unwrap_or_default()
                .contains(&state.favorites[0].url),
            "Favorite full URL must be available through native help/F1: {:?}",
            help.label()
        );
    }
}
