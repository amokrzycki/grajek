use gpui_kit::{Focusable, StatefulInteractiveElement};
use std::collections::HashMap;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    Context, ElementId, FocusHandle, InteractiveElement, IntoElement, ListAlignment,
    ListSizingBehavior, ListState, ParentElement, Styled, TestSupportExt, Window, div, list, px,
};

use super::help::HelpExt;
use super::{PlayerApp, Section};
use gpui_kit::component::Disableable;
use gpui_kit::component::{ActiveTheme, Sizable, button::ButtonVariants, popover::Popover};
use gpui_kit::prelude::FluentBuilder;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum TrackTarget {
    Queue(u64),
    History(String),
}

pub(super) struct ListFocus {
    queue: Option<(usize, u64)>,
    history: Option<(usize, String)>,
}

pub(super) const ROW_HEIGHT: gpui_kit::Pixels = px(44.);

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
        self.state = self.state.clone().with_uniform_item_height(ROW_HEIGHT);
        self.state.scroll_to(offset);
    }
}

impl PlayerApp {
    /// Both collections share row metrics, hover, native focus scope and title space.
    pub(super) fn track_row(
        &self,
        id: ElementId,
        focus: &FocusHandle,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let theme = cx.theme();
        div()
            .id(id)
            .w_full()
            .tab_group()
            .track_focus(focus)
            .tab_stop(false)
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_2()
            .h(ROW_HEIGHT)
            .rounded(theme.radius)
            .text_sm()
            .when(
                self.focus.keyboard && focus.contains_focused(window, cx),
                |row| row.bg(theme.muted),
            )
            .hover(|row| row.bg(theme.list_hover))
    }

    pub(super) fn row_menu(
        &self,
        section: Section,
        row: usize,
        key: u64,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let prefix = match section {
            Section::Queue => "queue-more",
            Section::History => "history-more",
        };
        let target = match section {
            Section::Queue => TrackTarget::Queue(key),
            Section::History => TrackTarget::History(self.state.history[row].url.clone()),
        };
        let ready = self.state.controller_ready;
        let actions: &[(&str, &str, bool)] = match section {
            Section::Queue => &[
                ("queue-up", "Move up", ready && row > 0),
                (
                    "queue-down",
                    "Move down",
                    ready && row + 1 < self.state.queue.len(),
                ),
                ("queue-remove", "Remove from queue", ready),
            ],
            Section::History => &[
                ("history-queue", "Add to queue", self.state.can_enqueue()),
                ("history-remove", "Forget from History", ready),
            ],
        };
        let mut content = div()
            .id("track-options-menu")
            .role(gpui_kit::Role::Group)
            .aria_label("Track options")
            .flex()
            .flex_col()
            .w(px(200.))
            .gap_1()
            .on_key_down(Self::menu_key);
        for &(name, title, enabled) in actions {
            let keyboard_target = target.clone();
            let pointer_target = target.clone();
            content = content.child(
                self.button((name, key))
                    .label(title)
                    .child(div().flex_1())
                    .ghost()
                    .small()
                    .w_full()
                    .disabled(!enabled)
                    .key_context("PlayerMenuControl")
                    .on_action(cx.listener(
                        move |this, _: &super::controls::Activate, window, cx| {
                            if enabled {
                                this.track_operation(&keyboard_target, name, window, cx);
                            }
                        },
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if enabled {
                            this.track_operation(&pointer_target, name, window, cx);
                        }
                    })),
            );
        }
        let open = self.row_menu.as_ref() == Some(&target);
        Popover::new((prefix, key))
            .open(open)
            .anchor(gpui_kit::Anchor::TopRight)
            .on_open_change(cx.listener(move |this, open: &bool, window, cx| {
                this.row_menu = open.then(|| target.clone());
                this.queue_options_open = false;
                this.help.dismiss(cx);
                if *open && window.last_input_was_keyboard() {
                    let target = target.clone();
                    cx.on_next_frame(window, move |this, window, cx| {
                        if this.row_menu.as_ref() == Some(&target) {
                            window.focus_next(cx);
                        }
                    });
                }
                cx.notify();
            }))
            .trigger(
                self.button((prefix, key))
                    .icon(gpui_kit::assets::IconName::Ellipsis)
                    .ghost()
                    .small()
                    .accessibility_label(self.track_menu_label(section, row))
                    .help(
                        if section == Section::Queue {
                            "Track options · Alt+Up/Down to reorder · Delete to remove"
                        } else {
                            "Track options · add to queue or forget from History"
                        },
                        &self.help,
                    )
                    .key_context("PlayerList")
                    .on_action(cx.listener(move |this, _: &Tab, window, cx| {
                        this.list_tab(section, row, 1, false, window, cx);
                    }))
                    .on_action(cx.listener(move |this, _: &TabPrev, window, cx| {
                        this.list_tab(section, row, 1, true, window, cx);
                    })),
            )
            .child(content.test_support())
    }

    fn track_menu_label(&self, section: Section, row: usize) -> String {
        match section {
            Section::Queue => format!(
                "Track options for {}. Move up, move down, remove from queue",
                self.state.queue[row].label()
            ),
            Section::History => {
                let entry = &self.state.history[row];
                let title = entry
                    .title
                    .as_deref()
                    .unwrap_or_else(|| crate::queue::video_id(&entry.url));
                format!("Track options for {title}. Add to queue or forget from History")
            }
        }
    }

    fn track_operation(
        &mut self,
        target: &TrackTarget,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.row_menu = None;
        let (section, row) = match target {
            TrackTarget::Queue(id) => (
                Section::Queue,
                self.state.queue.iter().position(|item| item.id == *id),
            ),
            TrackTarget::History(url) => (
                Section::History,
                self.state.history.iter().position(|item| item.url == *url),
            ),
        };
        if let Some(row) = row {
            self.focus_row_action(section, row, 1, window, cx);
        }
        match (target, name) {
            (TrackTarget::Queue(id), "queue-up" | "queue-down") => {
                self.send(crate::player::Command::Shift {
                    id: *id,
                    up: name == "queue-up",
                });
            }
            (TrackTarget::Queue(id), "queue-remove") => {
                self.send(crate::player::Command::Dequeue(*id));
            }
            (TrackTarget::History(url), "history-queue") => self.enqueue_url(url.clone()),
            (TrackTarget::History(url), "history-remove") => {
                self.send(crate::player::Command::ForgetHistory(url.clone()));
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn list_focus(
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
        ListFocus { queue, history }
    }

    pub(super) fn restore_list_focus(
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

    pub(super) fn list_key(
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
        let navigation = match section {
            Section::Queue => &self.queue_scroll,
            Section::History => &self.history_scroll,
        };
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
    pub(super) fn menu_key(
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
                list(scroll.clone(), move |position, window, cx| {
                    view.update(cx, |app, cx| match section {
                        Section::Queue => app
                            .queue_row(
                                position,
                                position + 1 == app.state.queue.len(),
                                &app.state.queue[position],
                                window,
                                cx,
                            )
                            .into_any_element(),
                        Section::History => app
                            .history_row(position, &app.state.history[position], window, cx)
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

    pub(super) fn row_actions(&self, section: Section, _row: usize) -> [bool; 2] {
        match section {
            Section::Queue => [false, true],
            Section::History => [self.play_enabled(), true],
        }
    }

    /// Focuses one action of a row, revealing the row first (it may be outside the viewport).
    pub(super) fn focus_row_action(
        &self,
        section: Section,
        row: usize,
        action: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let navigation = match section {
            Section::Queue => &self.queue_scroll,
            Section::History => &self.history_scroll,
        };
        navigation.state.scroll_to_reveal_item(row);
        cx.notify();
        let focus = navigation.rows[row].1.clone();
        let steps = self.row_actions(section, row)[..=action]
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
            if !self.row_actions(section, next_row)[next % 2] {
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
