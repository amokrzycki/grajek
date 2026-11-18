use gpui_kit::component::{
    ActiveTheme, IndexPath, Sizable,
    select::{Select, SelectDelegate, SelectEvent, SelectItem, SelectState},
    tab::{Tab, TabBar},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, TestSupportExt, Window, div, px,
};

use super::{PlayerApp, Section};
use crate::player::State;

pub(super) const COLLECTION_BREAKPOINT: gpui_kit::Pixels = px(500.);

impl Section {
    pub(super) fn index(self) -> usize {
        match self {
            Self::Queue => 0,
            Self::History => 1,
            Self::Favorites => 2,
        }
    }

    pub(super) fn from_index(index: usize) -> Self {
        match index {
            1 => Self::History,
            2 => Self::Favorites,
            _ => Self::Queue,
        }
    }

    fn label(self, state: &State) -> String {
        match self {
            Self::Queue => {
                let mut label = format!("Up next · {}", state.queue.len());
                if state.queue.len() >= crate::queue::QUEUE_LIMIT {
                    label.push_str(" · Full");
                }
                label
            }
            Self::History => format!("History · {}", state.history.len()),
            Self::Favorites => {
                let mut label = format!("Favorites · {}", state.favorites.len());
                if state.favorites.len() >= crate::store::FAVORITES_LIMIT {
                    label.push_str(" · Full");
                }
                label
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct CollectionChoice {
    index: usize,
    title: SharedString,
}

impl SelectItem for CollectionChoice {
    type Value = usize;
    fn title(&self) -> SharedString {
        self.title.clone()
    }
    fn value(&self) -> &usize {
        &self.index
    }
}

pub(super) struct CollectionChoices(Vec<CollectionChoice>);

impl SelectDelegate for CollectionChoices {
    type Item = CollectionChoice;
    fn items_count(&self, _: usize) -> usize {
        self.0.len()
    }
    fn item(&self, ix: IndexPath) -> Option<&CollectionChoice> {
        self.0.get(ix.row)
    }
    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        CollectionChoice: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.0
            .iter()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }
}

pub(super) type CollectionSelect = Entity<SelectState<CollectionChoices>>;

pub(super) fn selector(window: &mut Window, cx: &mut App) -> CollectionSelect {
    cx.new(|cx| SelectState::new(CollectionChoices(Vec::new()), None, window, cx))
}

impl PlayerApp {
    pub(super) fn subscribe_collection_selector(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.subscriptions
            .push(cx.observe_window_bounds(window, |this, window, cx| {
                if window.viewport_size().width > COLLECTION_BREAKPOINT
                    && this
                        .collection_select
                        .read(cx)
                        .focus_handle(cx)
                        .contains_focused(window, cx)
                {
                    this.help.dismiss(cx);
                    window.focus(&this.input.read(cx).focus_handle(cx), cx);
                }
            }));
        self.subscriptions.push(cx.subscribe_in(
            &self.collection_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(index)) = event {
                    this.activate_section(Section::from_index(*index), window, cx);
                }
            },
        ));
    }

    pub(super) fn sync_collection_selector(&self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.state;
        let section = self.section;
        let choices = CollectionChoices(
            [Section::Queue, Section::History, Section::Favorites]
                .into_iter()
                .map(|section| CollectionChoice {
                    index: section.index(),
                    title: section.label(state).into(),
                })
                .collect(),
        );
        self.collection_select.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_value(&section.index(), window, cx);
        });
    }

    pub(super) fn activate_section(
        &mut self,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_search(window, cx);
        let list_had_focus = self.queue_options_open
            || self.row_menu.is_some()
            || self
                .queue_scroll
                .rows
                .iter()
                .chain(&self.history_scroll.rows)
                .chain(&self.favorites_scroll.rows)
                .any(|(_, focus)| focus.contains_focused(window, cx));
        self.select_section(section, cx);
        self.sync_collection_selector(window, cx);
        if list_had_focus {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn select_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.row_menu = None;
        self.queue_options_open = false;
        self.confirm_clear = false;
        cx.notify();
    }

    pub(super) fn sections(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        if self.searching() {
            return self.search_collection(cx);
        }
        let narrow = window.viewport_size().width <= COLLECTION_BREAKPOINT;
        div()
            .id("queue-region")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(self.collection_header(narrow, window, cx))
            .child(match self.section {
                Section::Queue => self.queue_list(cx).into_any_element(),
                Section::History => self.history_list(cx).into_any_element(),
                Section::Favorites => self.favorite_list(cx).into_any_element(),
            })
            .into_any_element()
    }

    fn collection_header(
        &self,
        narrow: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let navigation = if narrow {
            self.collection_chooser(window, cx).into_any_element()
        } else {
            self.collection_tabs(cx).into_any_element()
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(navigation)
            .when(self.section == Section::History, |header| {
                header.child(self.history_header(cx))
            })
    }

    fn collection_tabs(&self, cx: &Context<Self>) -> impl IntoElement {
        TabBar::new("sections")
            .underline()
            .small()
            .selected_index(self.section.index())
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                this.activate_section(Section::from_index(*index), window, cx);
            }))
            .child(Tab::new().label(Section::Queue.label(&self.state)))
            .child(Tab::new().label(Section::History.label(&self.state)))
            .child(Tab::new().label(Section::Favorites.label(&self.state)))
    }

    fn collection_chooser(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let focused = self
            .collection_select
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        Select::new(&self.collection_select)
            .appearance(false)
            .bg(cx.theme().muted)
            .text_color(cx.theme().foreground)
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(self.focus.editing_border(focused, cx))
            .id("collection-selector")
            .w_full()
            .h(px(32.))
            .accessibility_label("Collection")
            .placeholder("Choose a collection")
            .menu_max_h(px(224.))
    }
}
