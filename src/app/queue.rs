use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Styled,
    TestSupportExt, Window, div, px,
};

use super::{PlayerApp, Section, label_with_tooltip};
use crate::{player::Command, queue::QueueItem};

impl PlayerApp {
    pub(super) fn enqueue_url(&self, url: String) {
        if self.state.can_enqueue() {
            self.send(Command::Enqueue(url));
        }
    }

    /// Alt+Up / Alt+Down move the entry holding focus; Delete removes it. Handles are keyed by
    /// entry id, so focus follows a moved entry.
    fn queue_key(&self, id: u64, position: usize, event: &KeyDownEvent) {
        let modifiers = &event.keystroke.modifiers;
        if !self.state.controller_ready {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" if modifiers.alt && position > 0 => {
                self.send(Command::Shift { id, up: true });
            }
            "down" if modifiers.alt && position + 1 < self.state.queue.len() => {
                self.send(Command::Shift { id, up: false });
            }
            "delete" if !modifiers.alt && !modifiers.control && !modifiers.shift => {
                self.send(Command::Dequeue(id));
            }
            _ => {}
        }
    }

    pub(super) fn queue_row(
        &self,
        position: usize,
        _last: bool,
        item: &QueueItem,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        #[cfg(test)]
        let _row_timer = super::tests::animation_fixes::RowTimer::start();
        let theme = cx.theme();
        let id = item.id;
        let next = position == 0;
        self.track_row(
            ("queue-row", id).into(),
            &self.queue_scroll.rows[position].1,
            window,
            cx,
        )
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if this.row_menu.is_none() {
                this.list_key(Section::Queue, position, event, window, cx);
                this.queue_key(id, position, event);
            }
        }))
        .child(
            div()
                .w(px(24.))
                .flex_none()
                .text_right()
                .text_xs()
                .font_family(theme.mono_font_family.clone())
                .text_color(theme.muted_foreground)
                .child(format!("{}", position + 1)),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .font_weight(if next {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if next {
                    theme.foreground
                } else {
                    theme.foreground.opacity(0.86)
                })
                .child(label_with_tooltip(
                    ("queue-title", id),
                    item.label().to_owned(),
                    item.title.clone().unwrap_or_else(|| {
                        if item.title_unavailable {
                            format!(
                                "Could not load the title. Playback can still be attempted. {}",
                                item.url
                            )
                        } else {
                            format!("Waiting for YouTube metadata. {}", item.url)
                        }
                    }),
                    &self.help,
                )),
        )
        .child(self.row_menu(Section::Queue, position, id, cx))
        .test_support()
    }

    pub(super) fn queue_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.state.queue.len();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(if count == 0 {
                Self::empty("queue-empty", "Nothing up next.", cx).into_any_element()
            } else {
                self.scroll_list(Section::Queue, cx).into_any_element()
            })
    }
}
