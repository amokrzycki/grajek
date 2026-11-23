//! Time, label and layout vocabulary shared by every player surface.

use std::time::Duration;

use gpui_kit::{
    ElementId, InteractiveElement, IntoElement, ParentElement, Pixels, StatefulInteractiveElement,
    Styled, TestSupportExt, Window, div, px,
};

use super::ui::Help;
use super::ui::help::HelpExt;

/// Below this window height the player drops to its compact type scale and spacing.
pub(super) const COMPACT_HEIGHT: Pixels = px(480.);
/// At or below this width the collection tabs become a chooser.
pub(super) const COLLECTION_BREAKPOINT: Pixels = px(500.);
pub(crate) const WINDOW_MIN_WIDTH: Pixels = px(420.);
pub(crate) const WINDOW_MIN_HEIGHT: Pixels = COMPACT_HEIGHT;

pub(super) fn heading_size(window: &Window) -> Pixels {
    if window.viewport_size().height <= COMPACT_HEIGHT {
        px(20.)
    } else {
        px(24.)
    }
}

pub(super) fn clock(position: Duration) -> String {
    let seconds = position.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// `01:24 / 03:45`, or `01:24 (duration unavailable)` when yt-dlp gave no duration.
pub(super) fn format_progress(position: Duration, duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("{} / {}", clock(position), clock(duration)),
        None => format!("{} (duration unavailable)", clock(position)),
    }
}

/// A one-line, ellipsized label whose full text appears in a tooltip.
pub(super) fn label(id: impl Into<ElementId>, text: String, help: &Help) -> impl IntoElement {
    let tip = text.clone();
    label_with_tooltip(id, text, tip, help)
}

pub(super) fn label_with_tooltip(
    id: impl Into<ElementId>,
    text: String,
    tip: String,
    help: &Help,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .truncate()
        .aria_label(text.clone())
        .child(text)
        .help(tip, help)
        .test_support()
}
