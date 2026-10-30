//! GPUI Kit buttons don't request scroll-to-focus; keep preferences keyboard-visible.
use gpui_kit::base::ElementExt;
use gpui_kit::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, RenderOnce, ScrollHandle,
    Styled, Window, div, px,
};

#[derive(IntoElement)]
pub(super) struct Reveal {
    id: &'static str,
    content: AnyElement,
    scroll: ScrollHandle,
}

pub(super) trait RevealExt: IntoElement + Sized + 'static {
    fn reveal(self, id: &'static str, scroll: &ScrollHandle) -> Reveal {
        Reveal {
            id,
            content: self.into_any_element(),
            scroll: scroll.clone(),
        }
    }
}
impl<T: IntoElement + 'static> RevealExt for T {}

impl RenderOnce for Reveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id, cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        div()
            .id(self.id)
            .relative()
            .flex()
            .flex_col()
            .track_focus(&focus)
            .tab_stop(false)
            .child(self.content)
            .on_prepaint(move |bounds, window, cx| {
                if !focus.contains_focused(window, cx) {
                    return;
                }
                let viewport = self.scroll.bounds();
                let delta = if bounds.top() < viewport.top() + px(4.) {
                    viewport.top() + px(4.) - bounds.top()
                } else if bounds.bottom() > viewport.bottom() - px(4.) {
                    viewport.bottom() - px(4.) - bounds.bottom()
                } else {
                    px(0.)
                };
                if delta != px(0.) {
                    let mut offset = self.scroll.offset();
                    offset.y = (offset.y + delta).clamp(-self.scroll.max_offset().y, px(0.));
                    if offset != self.scroll.offset() {
                        self.scroll.set_offset(offset);
                        window.refresh();
                    }
                }
            })
    }
}
