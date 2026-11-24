//! Popover activation keys and the shared menu enter transition.

use gpui_kit::base::animation::{EffectTransition, ease_out_cubic};
use gpui_kit::{AnyElement, App, ElementId, IntoElement, Styled, px};

use super::super::motion::MENU_ENTER;

gpui_kit::actions!(player_menu, [Activate]);

pub(in crate::app) fn init(cx: &mut App) {
    // Popover's Enter binding otherwise shadows the native button activation.
    cx.bind_keys([
        gpui_kit::KeyBinding::new("enter", Activate, Some("PlayerMenuControl")),
        gpui_kit::KeyBinding::new("space", Activate, Some("PlayerMenuControl")),
    ]);
}

/// Menus mount only while open, so element state replays on every open.
pub(in crate::app) fn menu_enter(
    element: impl IntoElement + Styled + 'static,
    id: impl Into<ElementId>,
    cx: &App,
) -> AnyElement {
    if cx.reduce_motion() {
        return element.into_any_element();
    }
    EffectTransition::new(MENU_ENTER)
        .ease(ease_out_cubic)
        .slide_y(px(-4.), px(0.))
        .fade(0., 1.)
        .apply(element, id)
        .into_any_element()
}
