//! Offline native cases for Queue next and Move to next. The window is driven through real
//! input; the controller is a scripted snapshot source, so these prove window behavior only.
use super::*;
use crate::player::{EnqueueResult, Placement};
use gpui_kit::{AsyncApp, BackgroundExecutor};
use std::path::Path;

pub(super) const CASES: [&str; 15] = [
    "qn-playing",
    "qn-paused",
    "qn-connecting",
    "qn-resolving",
    "qn-idle",
    "qn-repeat-track",
    "qn-history",
    "qn-move",
    "qn-move-first",
    "qn-move-stale",
    "qn-move-deep",
    "qn-full",
    "qn-keyboard",
    "qn-keyboard-move",
    "qn-keyboard-history",
];

const LINK: &str = "https://youtu.be/aqz-KE-bpKQ";
const NOTE: &str = "Repeat Track is on; press Next to play it.";

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
    reason = "opt-in native evidence fails on any window or capture error"
)]
impl Fixture<'_> {
    async fn settle(&self, millis: u64) {
        self.executor.timer(Duration::from_millis(millis)).await;
    }

    fn status(&self, cx: &mut AsyncApp) -> String {
        cx.update_window(self.window, |_, window, _| {
            window
                .find("link-status")
                .label()
                .unwrap_or_default()
                .to_owned()
        })
        .expect("status line")
    }

    fn disabled(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            let element = window.find(id);
            element
                .disabled()
                .unwrap_or_else(|| element.focused().is_none())
        })
        .expect("control state")
    }

    fn input_focus(&self, cx: &mut AsyncApp) {
        cx.update_window(self.window, |_, window, cx| {
            self.view.update(cx, |app, cx| {
                window.focus(&app.input.read(cx).focus_handle(cx), cx);
            });
        })
        .expect("focus the editor");
    }

    fn tab_to(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(self.window, |_, window, cx| {
            self.view.update(cx, |app, cx| {
                window.focus(&app.input.read(cx).focus_handle(cx), cx);
            });
            for _ in 0..60 {
                window.render_frame(cx);
                if window.find(id.clone()).focused() == Some(true) {
                    return;
                }
                window.press("tab", cx);
            }
            assert_eq!(
                window.find(id).focused(),
                Some(true),
                "keyboard focus reached the control"
            );
        })
        .expect("tab to control");
    }

    fn open_row_menu(&self, cx: &mut AsyncApp, trigger: impl Into<ElementId>) {
        let trigger = trigger.into();
        cx.update_window(self.window, |_, window, cx| {
            window.click(trigger, cx);
            window.render_frame(cx);
        })
        .expect("open row menu");
    }

    fn click(&self, cx: &mut AsyncApp, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(self.window, |_, window, cx| window.click(id, cx))
            .expect("click");
    }

    fn press(&self, cx: &mut AsyncApp, key: &str) {
        cx.update_window(self.window, |_, window, cx| {
            window.press(key, cx);
            window.render_frame(cx);
        })
        .expect("press");
    }

    fn record(&self, name: &str, text: &str) {
        std::fs::write(
            self.directory
                .join(format!("{name}-{:.0}.interaction.txt", self.width)),
            format!("{text}\nOffline scripted snapshots; no network, Discord or audio.\n"),
        )
        .expect("interaction evidence");
    }

    fn next_request(&mut self, expected: &str) -> u64 {
        let Ok(Command::EnqueueRequest {
            id,
            value,
            placement: Placement::Next,
        }) = self.receiver.try_recv()
        else {
            panic!("expected one Queue next request");
        };
        assert_eq!(value, expected);
        assert!(self.receiver.try_recv().is_err(), "exactly one command");
        id
    }

    fn next_move(&mut self, entry: u64) -> u64 {
        let Ok(Command::MoveToNext { request_id, id }) = self.receiver.try_recv() else {
            panic!("expected one Move to next request");
        };
        assert_eq!(id, entry);
        assert!(self.receiver.try_recv().is_err(), "exactly one command");
        request_id
    }
}

impl Fixture<'_> {
    #[expect(clippy::future_not_send, reason = "GPUI foreground task")]
    pub(super) async fn drive(&mut self, name: &str, base: &State, cx: &mut AsyncApp) -> usize {
        drive_case(name, base, self, cx).await
    }

    pub(super) fn full(&self, base: &State, cx: &mut AsyncApp) {
        full_case(base, self, cx);
    }
}

fn answered(mut state: State, request_id: u64, item_id: Option<u64>) -> State {
    state.enqueue_result = Some(EnqueueResult {
        request_id,
        item_id,
    });
    state
}

fn inserted(base: &State, url: &str, title: &str) -> (State, u64) {
    let mut state = base.clone();
    let id = state.queue.iter().map(|item| item.id).max().unwrap_or(0) + 1;
    state.queue.insert(
        0,
        QueueItem {
            id,
            url: url.into(),
            title: Some(title.into()),
            title_unavailable: false,
        },
    );
    (state, id)
}

fn moved(base: &State, index: usize) -> State {
    let mut state = base.clone();
    let item = state.queue.remove(index);
    state.queue.insert(0, item);
    state
}

#[expect(
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::future_not_send,
    reason = "one opt-in native case driver keeps each entry point's real-input script together"
)]
pub(super) async fn drive_case(
    name: &str,
    base: &State,
    fixture: &mut Fixture<'_>,
    cx: &mut AsyncApp,
) -> usize {
    let ids: Vec<u64> = base.queue.iter().map(|item| item.id).collect();
    let mut count = base.queue.len();
    match name {
        "qn-move-first" => {
            fixture.open_row_menu(cx, ("queue-more", ids[0]));
            assert!(
                fixture.disabled(cx, ("queue-next", ids[0])),
                "first row is already next"
            );
            assert!(fixture.receiver.try_recv().is_err());
            fixture.record(name, "Row 0 menu: Move to next disabled, no command sent.");
        }
        "qn-move" | "qn-keyboard-move" | "qn-move-stale" | "qn-move-deep" => {
            let index = match name {
                "qn-move" if fixture.width > 500. => 3,
                "qn-move-deep" => 40,
                _ => 2,
            };
            let entry = ids[index];
            if name == "qn-move-deep" {
                cx.update_window(fixture.window, |_, _, cx| {
                    fixture.view.update(cx, |app, _| {
                        app.queue_scroll.state.scroll_to_reveal_item(index);
                    });
                })
                .expect("scroll to the deep row");
                fixture.settle(300).await;
            }
            if name == "qn-keyboard-move" {
                fixture.tab_to(cx, ("queue-more", entry));
                fixture.press(cx, "enter");
                fixture.settle(100).await;
                fixture.press(cx, "enter");
            } else {
                fixture.open_row_menu(cx, ("queue-more", entry));
                fixture.click(cx, ("queue-next", entry));
            }
            fixture.settle(150).await;
            let request = fixture.next_move(entry);
            assert!(
                !fixture.status(cx).starts_with("Moved to next"),
                "never optimistic"
            );
            let reply = if name == "qn-move-stale" {
                let mut gone = base.clone();
                gone.queue.remove(index);
                count = gone.queue.len();
                answered(gone, request, None)
            } else {
                answered(moved(base, index), request, Some(entry))
            };
            fixture.snapshots.send_replace(reply);
            fixture.settle(500).await;
            if name == "qn-move-stale" {
                assert_eq!(fixture.status(cx), "", "a stale move is silent");
                fixture.record(
                    name,
                    "Move for a consumed entry: acknowledged without a claim, no error text.",
                );
            } else {
                assert!(fixture.status(cx).starts_with("Moved to next · "));
                if name != "qn-move" {
                    let (focused, visible) = cx
                        .update_window(fixture.window, |_, window, _| {
                            let row = window.find(("queue-row", entry)).bounds();
                            let list = window.find("queue-list").bounds();
                            (
                                window.find(("queue-more", entry)).focused() == Some(true),
                                row.top() >= list.top() && row.bottom() <= list.bottom(),
                            )
                        })
                        .expect("moved row state");
                    assert!(focused && visible, "{name}: row 0 is visible and focused");
                }
                fixture.record(name, "Move to next: one intent, claim only after the acknowledgement; moved row first (visible and focused where the focus was in the row).");
            }
        }
        "qn-history" | "qn-keyboard-history" => {
            let index = if name == "qn-history" { 2 } else { 0 };
            if name == "qn-history" {
                fixture.open_row_menu(cx, ("history-more", index));
                fixture.click(cx, ("history-next", index));
            } else {
                fixture.tab_to(cx, ("history-more", index));
                fixture.press(cx, "enter");
                fixture.settle(100).await;
                fixture.press(cx, "down");
                fixture.press(cx, "enter");
            }
            fixture.settle(150).await;
            let url = base.history[index].url.clone();
            let request = fixture.next_request(&url);
            assert!(
                !fixture.status(cx).starts_with("Queued next"),
                "never optimistic"
            );
            let (reply, _) = inserted(
                base,
                &url,
                "Played history entry (acknowledged by the controller)",
            );
            count = reply.queue.len();
            fixture.snapshots.send_replace(answered(
                reply.clone(),
                request,
                Some(reply.queue[0].id),
            ));
            fixture.settle(500).await;
            assert!(fixture.status(cx).starts_with("Queued next · "));
            fixture.record(
                name,
                "History Queue next: one request, claim only after the acknowledgement.",
            );
        }
        _ => {
            fixture.input_focus(cx);
            cx.update_window(fixture.window, |_, window, cx| window.input(LINK, cx))
                .expect("type a link");
            if name == "qn-keyboard" {
                fixture.tab_to(cx, "enqueue-next");
                fixture.press(cx, "enter");
            } else {
                fixture.click(cx, "enqueue-next");
            }
            fixture.settle(150).await;
            let request = fixture.next_request(LINK);
            assert!(
                !fixture.status(cx).starts_with("Queued next"),
                "never optimistic"
            );
            let (reply, id) = inserted(
                base,
                LINK,
                "Aqua Mundi — Live from the Funkhaus (acknowledged by the controller)",
            );
            count = reply.queue.len();
            fixture
                .snapshots
                .send_replace(answered(reply, request, Some(id)));
            fixture.settle(500).await;
            let status = fixture.status(cx);
            assert!(status.starts_with("Queued next · "), "{name}: {status}");
            assert_eq!(status.contains(NOTE), name == "qn-repeat-track");
            fixture.record(name, "Composer Queue next: one request, active or preparing track untouched, claim only after the acknowledgement.");
        }
    }
    count
}

pub(super) fn full_case(base: &State, fixture: &Fixture<'_>, cx: &mut AsyncApp) {
    let ids: Vec<u64> = base.queue.iter().map(|item| item.id).collect();
    assert!(
        fixture.disabled(cx, "enqueue-next"),
        "no free slot for Queue next"
    );
    assert!(
        !fixture.disabled(cx, "enqueue"),
        "Add to queue keeps its capacity-blind gate"
    );
    fixture.open_row_menu(cx, ("queue-more", ids[1]));
    assert!(
        !fixture.disabled(cx, ("queue-next", ids[1])),
        "Move to next works when full"
    );
    fixture.record(
        "qn-full",
        "Full queue: Queue next disabled, Move to next enabled.",
    );
}
