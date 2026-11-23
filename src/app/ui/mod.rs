//! Window chrome and the shared presentation helpers every surface draws with.

pub(super) mod focus;
pub(super) mod help;
pub(super) mod menu;
#[cfg(test)]
pub(in crate::app) mod row_probe;

mod chrome;

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    TestSupportExt, div,
};

use super::PlayerApp;

pub(super) use focus::FocusPresentation;
pub(super) use help::Help;

/// Keys the help overlay itself consumes; anything else dismisses it.
pub(super) fn help_key(key: &str) -> bool {
    matches!(key, "f1" | "escape")
}

pub(super) fn escape(key: &str) -> bool {
    key == "escape"
}

pub(super) fn empty(
    id: &'static str,
    text: &'static str,
    cx: &Context<PlayerApp>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .pt_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .aria_label(text)
        .child(text)
        .test_support()
}
