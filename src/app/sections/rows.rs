//! Shared row chrome and the per-section row dispatch.

use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, Styled,
    Window, div, px,
};
use std::time::Duration;

use super::super::PlayerApp;
use super::super::fmt::clock;
use super::Section;
use super::lists::ROW_HEIGHT;

impl Section {
    pub(super) fn row_ids(self, app: &PlayerApp) -> Vec<ElementId> {
        match self {
            Self::Queue => app
                .state
                .queue
                .iter()
                .map(|item| ("queue-row", item.id).into())
                .collect(),
            Self::History => (0..app.state.history.len())
                .map(|index| ("history-row", index).into())
                .collect(),
            Self::Favorites => app
                .state
                .favorites
                .iter()
                .map(|entry| format!("favorite-row:{}", entry.url).into())
                .collect(),
        }
    }

    pub(super) fn row(
        self,
        app: &PlayerApp,
        index: usize,
        window: &Window,
        cx: &Context<PlayerApp>,
    ) -> gpui_kit::AnyElement {
        match self {
            Self::Queue => app
                .queue_row(index, &app.state.queue[index], window, cx)
                .into_any_element(),
            Self::History => app
                .history_row(index, &app.state.history[index], window, cx)
                .into_any_element(),
            Self::Favorites => app
                .favorite_row(index, &app.state.favorites[index], window, cx)
                .into_any_element(),
        }
    }

    pub(super) fn row_actions(self, app: &PlayerApp) -> [bool; 2] {
        match self {
            Self::Queue => [false, true],
            Self::History | Self::Favorites => [app.play_enabled(), true],
        }
    }
}

impl PlayerApp {
    /// All three collections share row metrics, hover and native focus scope.
    pub(in crate::app) fn track_row(
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

    pub(in crate::app) fn duration_cell(
        duration: Duration,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex_none()
            .w(px(40.))
            .text_right()
            .whitespace_nowrap()
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(clock(duration))
    }
}
