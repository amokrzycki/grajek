use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt, div,
};

use super::{PlayerApp, label};
use crate::{
    player::Command,
    queue::{QUEUE_LIMIT, QueueItem},
};

impl PlayerApp {
    pub(super) fn enqueue(&self, cx: &Context<Self>) {
        let url = self.input_text(cx);
        self.enqueue_url(url);
    }

    pub(super) fn enqueue_url(&self, url: String) {
        if self.state.can_enqueue() {
            self.send(Command::Enqueue(url));
        }
    }

    fn queue_row(
        position: usize,
        last: bool,
        item: &QueueItem,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let id = item.id;
        div()
            .id(("queue-row", id))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h_8()
            .rounded_md()
            .text_sm()
            .hover(|row| row.bg(theme.list_hover))
            .child(
                div()
                    .w_5()
                    .flex_none()
                    .text_right()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{}", position + 1)),
            )
            .child(label(("queue-title", id), item.label().to_owned()))
            .child(
                Button::new(("queue-up", id))
                    .icon(IconName::ArrowUp)
                    .ghost()
                    .xsmall()
                    .tooltip("Move up")
                    .disabled(position == 0)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: true });
                    })),
            )
            .child(
                Button::new(("queue-down", id))
                    .icon(IconName::ArrowDown)
                    .ghost()
                    .xsmall()
                    .tooltip("Move down")
                    .disabled(last)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: false });
                    })),
            )
            .child(
                Button::new(("queue-remove", id))
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip("Remove from queue")
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Dequeue(id));
                    })),
            )
            .test_support()
    }

    pub(super) fn queue_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.state.queue.len();
        let last = count.saturating_sub(1);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id("queue-count")
                            .child(format!(
                                "{count} pending{}",
                                if count >= QUEUE_LIMIT { " (full)" } else { "" }
                            ))
                            .test_support(),
                    )
                    .child(
                        Button::new("queue-clear")
                            .label("Clear queue")
                            .ghost()
                            .xsmall()
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, _, _| {
                                this.send(Command::ClearQueue);
                            })),
                    ),
            )
            .child(if count == 0 {
                Self::empty(
                    "queue-empty",
                    "The queue is empty. Paste a link and press Add to queue.",
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .id("queue-list")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scrollbar()
                    .children(self.state.queue.iter().enumerate().map(|(position, item)| {
                        Self::queue_row(position, position == last, item, cx)
                    }))
                    .into_any_element()
            })
    }
}
