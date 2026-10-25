use std::collections::HashMap;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    Context, ElementId, FocusHandle, InteractiveElement, IntoElement, ListAlignment,
    ListSizingBehavior, ListState, ParentElement, Styled, TestSupportExt, Window, div, list, px,
};

use super::{PlayerApp, Section};

gpui_kit::actions!(player_list, [Tab, TabPrev]);

pub(super) fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("tab", Tab, Some("PlayerList")),
        gpui_kit::KeyBinding::new("shift-tab", TabPrev, Some("PlayerList")),
    ]);
}

pub(super) struct ListNavigation {
    pub state: ListState,
    pub rows: Vec<(ElementId, FocusHandle)>,
}

impl ListNavigation {
    pub fn new() -> Self {
        Self {
            // shortcut: GPUI drops height hints on width changes; measure once per width until it retains them.
            state: ListState::new(0, ListAlignment::Top, px(0.)).measure_all(),
            rows: Vec::new(),
        }
    }

    fn sync(&mut self, ids: impl Iterator<Item = ElementId>, cx: &Context<PlayerApp>) {
        let ids: Vec<_> = ids.collect();
        // ponytail: Vec allocation unavoidable (Iterator consumed by comparison); sync fires only on state changes, not per-frame
        if self.rows.len() == ids.len() && self.rows.iter().map(|(id, _)| id).eq(ids.iter()) {
            return;
        }
        let offset = self.state.logical_scroll_top();
        let old_len = self.rows.len();
        let mut old: HashMap<_, _> = self.rows.drain(..).collect();
        self.rows = ids
            .into_iter()
            .map(|id| {
                let focus = old.remove(&id).unwrap_or_else(|| cx.focus_handle());
                (id, focus)
            })
            .collect();
        self.state.splice_focusable(
            0..old_len,
            self.rows.iter().map(|(_, focus)| Some(focus.clone())),
        );
        self.state = self.state.clone().with_uniform_item_height(px(36.));
        self.state.scroll_to(offset);
    }
}

impl PlayerApp {
    pub(super) fn sync_lists(&mut self, cx: &Context<Self>) {
        self.queue_scroll.sync(
            self.state
                .queue
                .iter()
                .map(|item| ("queue-row", item.id).into()),
            cx,
        );
        self.history_scroll.sync(
            (0..self.state.history.len()).map(|index| ("history-row", index).into()),
            cx,
        );
    }

    pub(super) fn scroll_list(&self, section: Section, cx: &Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let (id, scroll) = match section {
            Section::Queue => ("queue-list", &self.queue_scroll.state),
            Section::History => ("history-list", &self.history_scroll.state),
        };
        div()
            .id(id)
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .overflow_hidden()
            .child(
                list(scroll.clone(), move |position, _, cx| {
                    view.update(cx, |app, cx| match section {
                        Section::Queue => app
                            .queue_row(
                                position,
                                position + 1 == app.state.queue.len(),
                                &app.state.queue[position],
                                cx,
                            )
                            .into_any_element(),
                        Section::History => app
                            .history_row(position, &app.state.history[position], cx)
                            .into_any_element(),
                    })
                })
                .with_sizing_behavior(ListSizingBehavior::Auto)
                .flex_1()
                .min_w_0()
                .h_full()
                .mr(px(16.)),
            )
            .vertical_scrollbar(scroll)
            .test_support()
    }

    fn row_actions(&self, section: Section, row: usize) -> [bool; 3] {
        match section {
            Section::Queue => [
                self.state.controller_ready && row > 0,
                self.state.controller_ready && row + 1 < self.state.queue.len(),
                self.state.controller_ready,
            ],
            Section::History => [
                self.play_enabled(),
                self.state.can_enqueue(),
                self.state.controller_ready,
            ],
        }
    }

    /// Native buttons handle activation and within-row Tab; cross-row Tab must also
    /// render/reveal the logical neighbour, even when it was outside the viewport.
    pub(super) fn list_tab(
        &self,
        section: Section,
        row: usize,
        action: usize,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let navigation = match section {
            Section::Queue => &self.queue_scroll,
            Section::History => &self.history_scroll,
        };
        let mut next = row * 3 + action;
        loop {
            let Some(candidate) = (if backwards {
                next.checked_sub(1)
            } else {
                next.checked_add(1)
            }) else {
                if backwards {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                return;
            };
            next = candidate;
            let next_row = next / 3;
            if next_row >= navigation.rows.len() {
                if backwards {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                return;
            }
            if !self.row_actions(section, next_row)[next % 3] {
                continue;
            }
            navigation.state.scroll_to_reveal_item(next_row);
            cx.notify();
            if next_row == row {
                // ponytail: same-row focus relies on GPUI skipping disabled buttons; fragile if non-button focusable elements added to row
                if backwards {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                return;
            }
            let focus = navigation.rows[next_row].1.clone();
            let steps = self.row_actions(section, next_row)[..=next % 3]
                .iter()
                .filter(|enabled| **enabled)
                .count();
            window.focus(&focus, cx);
            cx.on_next_frame(window, move |_, window, cx| {
                if focus.is_focused(window) {
                    for _ in 0..steps {
                        window.focus_next(cx);
                    }
                }
            });
            return;
        }
    }
}
