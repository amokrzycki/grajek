//! Shared row/list navigation and per-section list rendering.

use gpui_kit::Focusable;
use std::collections::HashMap;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    Context, ElementId, FocusHandle, InteractiveElement, IntoElement, ListAlignment,
    ListSizingBehavior, ListState, ParentElement, Styled, TestSupportExt, Window, div, list, px,
};

use super::super::PlayerApp;
use super::Section;
use super::menu::TrackTarget;

pub(in crate::app) struct ListFocus {
    queue: Option<(usize, u64)>,
    history: Option<(usize, String)>,
    favorites: Option<(usize, String)>,
}

pub(in crate::app) const ROW_HEIGHT: gpui_kit::Pixels = px(44.);

gpui_kit::actions!(player_list, [Tab, TabPrev]);

pub(in crate::app) fn init(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        gpui_kit::KeyBinding::new("tab", Tab, Some("PlayerList")),
        gpui_kit::KeyBinding::new("shift-tab", TabPrev, Some("PlayerList")),
    ]);
}

pub(in crate::app) struct ListNavigation {
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
        self.state = self.state.clone().with_uniform_item_height(ROW_HEIGHT);
        self.state.scroll_to(offset);
    }
}

impl Section {
    fn navigation(self, app: &PlayerApp) -> &ListNavigation {
        match self {
            Self::Queue => &app.queue_scroll,
            Self::History => &app.history_scroll,
            Self::Favorites => &app.favorites_scroll,
        }
    }

    fn navigation_mut(self, app: &mut PlayerApp) -> &mut ListNavigation {
        match self {
            Self::Queue => &mut app.queue_scroll,
            Self::History => &mut app.history_scroll,
            Self::Favorites => &mut app.favorites_scroll,
        }
    }

    pub(super) fn list(self, app: &PlayerApp, cx: &Context<PlayerApp>) -> gpui_kit::AnyElement {
        match self {
            Self::Queue => app.queue_list(cx).into_any_element(),
            Self::History => app.history_list(cx).into_any_element(),
            Self::Favorites => app.favorite_list(cx).into_any_element(),
        }
    }
}

impl PlayerApp {
    pub(in crate::app) fn queue_row_focused(
        &self,
        id: u64,
        window: &Window,
        cx: &Context<Self>,
    ) -> bool {
        let row: ElementId = ("queue-row", id).into();
        self.queue_scroll
            .rows
            .iter()
            .any(|(key, focus)| *key == row && focus.contains_focused(window, cx))
    }

    pub(in crate::app) fn list_focus(
        &self,
        next: &crate::player::State,
        window: &Window,
        cx: &Context<Self>,
    ) -> ListFocus {
        let queue = self
            .queue_scroll
            .rows
            .iter()
            .position(|(_, focus)| focus.contains_focused(window, cx))
            .map(|index| (index, self.state.queue[index].id))
            .or_else(|| match &self.row_menu {
                Some(TrackTarget::Queue(id)) => self
                    .state
                    .queue
                    .iter()
                    .position(|item| item.id == *id)
                    .map(|index| (index, *id)),
                _ => None,
            });
        let history = if next.history == self.state.history {
            None
        } else {
            self.history_scroll
                .rows
                .iter()
                .position(|(_, focus)| focus.contains_focused(window, cx))
                .map(|index| (index, self.state.history[index].url.clone()))
                .or_else(|| match &self.row_menu {
                    Some(TrackTarget::History(url)) => self
                        .state
                        .history
                        .iter()
                        .position(|item| item.url == *url)
                        .map(|index| (index, url.clone())),
                    _ if next.history.is_empty()
                        && self.history_header_focus.contains_focused(window, cx) =>
                    {
                        Some((0, String::new()))
                    }
                    _ => None,
                })
        };
        let favorites = if next.favorites == self.state.favorites {
            None
        } else {
            self.favorites_scroll
                .rows
                .iter()
                .position(|(_, focus)| focus.contains_focused(window, cx))
                .map(|index| (index, self.state.favorites[index].url.clone()))
                .or_else(|| match &self.row_menu {
                    Some(TrackTarget::Favorite(url)) => self
                        .state
                        .favorites
                        .iter()
                        .position(|entry| entry.url == *url)
                        .map(|index| (index, url.clone())),
                    _ => None,
                })
        };
        ListFocus {
            queue,
            history,
            favorites,
        }
    }

    pub(in crate::app) fn restore_list_focus(
        &mut self,
        focus: ListFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let target = if let Some((index, url)) = focus.history
            && self.section == Section::History
        {
            self.row_menu = None;
            Some((
                Section::History,
                self.state
                    .history
                    .iter()
                    .position(|item| item.url == url)
                    .unwrap_or(index),
                self.state.history.len(),
            ))
        } else if let Some((index, id)) = focus.queue
            && self.section == Section::Queue
            && self.state.queue.iter().all(|item| item.id != id)
        {
            self.row_menu = None;
            Some((Section::Queue, index, self.state.queue.len()))
        } else if let Some((index, url)) = focus.favorites
            && self.section == Section::Favorites
        {
            if let Some(row) = self
                .state
                .favorites
                .iter()
                .position(|entry| entry.url == url)
            {
                self.favorites_scroll.state.scroll_to_reveal_item(row);
                return true;
            }
            self.row_menu = None;
            Some((Section::Favorites, index, self.state.favorites.len()))
        } else {
            None
        };
        let Some((section, row, count)) = target else {
            return false;
        };
        if count == 0 {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        } else {
            self.focus_row_action(section, row.min(count - 1), 1, window, cx);
        }
        true
    }

    pub(in crate::app) fn list_key(
        &self,
        section: Section,
        row: usize,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.alt
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.shift
        {
            return;
        }
        let navigation = section.navigation(self);
        let target = match event.keystroke.key.as_str() {
            "up" => row.saturating_sub(1),
            "down" => (row + 1).min(navigation.rows.len() - 1),
            "home" => 0,
            "end" => navigation.rows.len() - 1,
            _ => return,
        };
        self.focus_row_action(section, target, 1, window, cx);
        cx.stop_propagation();
    }

    /// Shared native popovers use Tab or arrows between their real, enabled buttons.
    pub(in crate::app) fn menu_key(
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) {
        if event.keystroke.modifiers.alt || event.keystroke.modifiers.control {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => window.focus_prev(cx),
            "down" => window.focus_next(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    pub(in crate::app) fn sync_lists(&mut self, cx: &Context<Self>) {
        for section in Section::ALL {
            let ids = section.row_ids(self);
            section.navigation_mut(self).sync(ids.into_iter(), cx);
        }
    }

    pub(in crate::app) fn scroll_list(
        &self,
        section: Section,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        let scroll = &section.navigation(self).state;
        let id = section.ids().list;
        div()
            .id(id)
            .flex_1()
            .min_h_0()
            .relative()
            .flex()
            .overflow_hidden()
            .child(
                list(scroll.clone(), move |position, window, cx| {
                    view.update(cx, |app, cx| section.row(app, position, window, cx))
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

    /// Focuses one action of a row, revealing the row first (it may be outside the viewport).
    pub(in crate::app) fn focus_row_action(
        &self,
        section: Section,
        row: usize,
        action: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let navigation = section.navigation(self);
        navigation.state.scroll_to_reveal_item(row);
        cx.notify();
        let focus = navigation.rows[row].1.clone();
        let steps = section.row_actions(self)[..=action]
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
    }

    /// Native buttons handle activation and within-row Tab; cross-row Tab must also
    /// render/reveal the logical neighbour, even when it was outside the viewport.
    pub(in crate::app) fn list_tab(
        &self,
        section: Section,
        row: usize,
        action: usize,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let navigation = section.navigation(self);
        let mut next = row * 2 + action;
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
            let next_row = next / 2;
            if next_row >= navigation.rows.len() {
                if backwards {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                return;
            }
            if !section.row_actions(self)[next % 2] {
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
            self.focus_row_action(section, next_row, next % 2, window, cx);
            return;
        }
    }
}
