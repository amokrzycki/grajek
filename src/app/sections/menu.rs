//! Track option menus: who each row menu targets and what each action sends.

use gpui_kit::component::{Disableable, Sizable, button::ButtonVariants, popover::Popover};
use gpui_kit::{
    Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, TestSupportExt, Window, div, px,
};

use super::super::PlayerApp;
use super::super::fmt::title_or_url_id;
use super::super::ui::help::HelpExt;
use super::super::ui::menu::menu_enter;
use super::Section;
use super::lists::{Tab, TabPrev};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) enum TrackTarget {
    Queue(u64),
    History(String),
    Favorite(String),
}

impl Section {
    fn menu_target(self, app: &PlayerApp, row: usize, key: u64) -> TrackTarget {
        match self {
            Self::Queue => TrackTarget::Queue(key),
            Self::History => TrackTarget::History(app.state.history[row].url.clone()),
            Self::Favorites => TrackTarget::Favorite(app.state.favorites[row].url.clone()),
        }
    }

    fn menu_id(self, app: &PlayerApp, name: &'static str, key: u64, row: usize) -> ElementId {
        if self == Self::Favorites {
            format!("{name}:{}", app.state.favorites[row].url).into()
        } else {
            (name, key).into()
        }
    }

    fn menu_actions(self, app: &PlayerApp, row: usize) -> Vec<(&'static str, String, bool)> {
        let ready = app.state.controller_ready;
        match self {
            Self::Queue => {
                let favorite = app.favorite_action_title(&app.state.queue[row].url);
                vec![
                    ("queue-next", "Move to next".into(), ready && row > 0),
                    ("queue-up", "Move up".into(), ready && row > 0),
                    (
                        "queue-down",
                        "Move down".into(),
                        ready && row + 1 < app.state.queue.len(),
                    ),
                    ("queue-remove", "Remove from queue".into(), ready),
                    ("queue-favorite", favorite, ready),
                ]
            }
            Self::History => {
                let favorite = app.favorite_action_title(&app.state.history[row].url);
                vec![
                    (
                        "history-queue",
                        "Add to queue".into(),
                        app.state.can_enqueue(),
                    ),
                    (
                        "history-next",
                        "Queue next".into(),
                        app.state.can_queue_next(),
                    ),
                    ("history-remove", "Forget from History".into(), ready),
                    ("history-favorite", favorite, ready),
                ]
            }
            Self::Favorites => vec![
                (
                    "favorite-queue",
                    "Add to queue".into(),
                    app.state.can_enqueue(),
                ),
                (
                    "favorite-next",
                    "Queue next".into(),
                    app.state.can_queue_next(),
                ),
                ("favorite-remove", "Remove from Favorites".into(), ready),
            ],
        }
    }

    fn menu_label(self, app: &PlayerApp, row: usize) -> String {
        match self {
            Self::Queue => format!(
                "Track options for {}. Move to next, move up, move down, remove from queue",
                app.state.queue[row].label()
            ),
            Self::History => {
                let entry = &app.state.history[row];
                let title = title_or_url_id(entry.title.as_deref(), &entry.url);
                format!(
                    "Track options for {title}. Add to queue, queue next or forget from History"
                )
            }
            Self::Favorites => {
                let entry = &app.state.favorites[row];
                let title = title_or_url_id(entry.title.as_deref(), &entry.url);
                format!(
                    "Track options for {title}. {}. Add to queue, queue next or remove from Favorites",
                    entry.url
                )
            }
        }
    }

    fn menu_help(self, label: &str) -> String {
        match self {
            Self::Queue => "Track options · Alt+Up/Down to reorder · Delete to remove".into(),
            Self::History => {
                "Track options · add to queue, queue next or forget from History".into()
            }
            Self::Favorites => label.to_owned(),
        }
    }
}

impl PlayerApp {
    pub(in crate::app) fn row_menu(
        &self,
        section: Section,
        row: usize,
        key: u64,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let ids = section.ids();
        let prefix = ids.menu;
        let target = section.menu_target(self, row, key);
        let id = |name: &'static str| section.menu_id(self, name, key, row);
        let actions = section.menu_actions(self, row);
        let mut content = div()
            .id("track-options-menu")
            .role(gpui_kit::Role::Group)
            .aria_label("Track options")
            .flex()
            .flex_col()
            .w(px(200.))
            .gap_1()
            .on_key_down(Self::menu_key);
        for (name, title, enabled) in actions {
            let keyboard_target = target.clone();
            let pointer_target = target.clone();
            content = content.child(
                self.focus
                    .button(id(name))
                    .label(title)
                    .child(div().flex_1())
                    .ghost()
                    .small()
                    .w_full()
                    .disabled(!enabled)
                    .key_context("PlayerMenuControl")
                    .on_action(cx.listener(
                        move |this, _: &super::super::ui::menu::Activate, window, cx| {
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
        let enter_id = id("track-options-menu-enter");
        let label = section.menu_label(self, row);
        let help = section.menu_help(&label);
        Popover::new(id(ids.popover))
            .open(open)
            .anchor(gpui_kit::Anchor::TopRight)
            .on_open_change(cx.listener(move |this, open: &bool, window, cx| {
                this.close_popups();
                this.row_menu = open.then(|| target.clone());
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
                self.focus
                    .button(id(prefix))
                    .icon(gpui_kit::assets::IconName::Ellipsis)
                    .ghost()
                    .small()
                    .accessibility_label(label)
                    .help(help, &self.help)
                    .key_context("PlayerList")
                    .on_action(cx.listener(move |this, _: &Tab, window, cx| {
                        this.list_tab(section, row, 1, false, window, cx);
                    }))
                    .on_action(cx.listener(move |this, _: &TabPrev, window, cx| {
                        this.list_tab(section, row, 1, true, window, cx);
                    })),
            )
            .child(menu_enter(content.test_support(), enter_id, cx))
    }

    pub(in crate::app) fn track_operation(
        &mut self,
        target: &TrackTarget,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popups();
        let (section, row) = match target {
            TrackTarget::Queue(id) => (
                Section::Queue,
                self.state.queue.iter().position(|item| item.id == *id),
            ),
            TrackTarget::History(url) => (
                Section::History,
                self.state.history.iter().position(|item| item.url == *url),
            ),
            TrackTarget::Favorite(url) => (
                Section::Favorites,
                self.state
                    .favorites
                    .iter()
                    .position(|entry| entry.url == *url),
            ),
        };
        if let Some(row) = row {
            self.focus_row_action(section, row, 1, window, cx);
        }
        match (target, name) {
            (TrackTarget::Queue(id), "queue-up" | "queue-down") => {
                if let Some(row) = row {
                    self.motion
                        .asked_to_move(*id, row, cx.background_executor().now());
                }
                self.send(crate::player::Command::Shift {
                    id: *id,
                    up: name == "queue-up",
                });
            }
            (TrackTarget::Queue(id), "queue-next") => self.move_to_next(*id),
            (TrackTarget::Queue(id), "queue-remove") => {
                self.send(crate::player::Command::Dequeue(*id));
            }
            (TrackTarget::Queue(id), "queue-favorite") => {
                if let Some(url) = self
                    .state
                    .queue
                    .iter()
                    .find(|item| item.id == *id)
                    .map(|item| item.url.clone())
                {
                    let saved = !self.is_favorite_url(&url);
                    self.send_set_favorite(url, saved, cx);
                }
            }
            (
                TrackTarget::History(url) | TrackTarget::Favorite(url),
                "history-queue" | "favorite-queue",
            ) => self.enqueue_url(url.clone(), crate::player::Placement::End),
            (
                TrackTarget::History(url) | TrackTarget::Favorite(url),
                "history-next" | "favorite-next",
            ) => self.enqueue_url(url.clone(), crate::player::Placement::Next),
            (TrackTarget::History(url), "history-favorite") => {
                let saved = !self.is_favorite_url(url);
                self.send_set_favorite(url.clone(), saved, cx);
            }
            (TrackTarget::History(url), "history-remove") => {
                self.send(crate::player::Command::ForgetHistory(url.clone()));
            }
            (TrackTarget::Favorite(url), "favorite-remove") => {
                self.send_set_favorite(url.clone(), false, cx);
            }
            _ => {}
        }
        cx.notify();
    }
}
