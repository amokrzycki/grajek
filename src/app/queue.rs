use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt, div, px,
};

use super::{PlayerApp, label_with_tooltip};
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
        &self,
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
            .flex_none()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(36.))
            .rounded(theme.radius)
            .text_sm()
            .hover(|row| row.bg(theme.list_hover))
            .child(
                div()
                    .w(px(24.))
                    .flex_none()
                    .text_right()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(format!("{:02}", position + 1)),
            )
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
            ))
            .child(
                Button::new(("queue-up", id))
                    .icon(IconName::ArrowUp)
                    .ghost()
                    .small()
                    .accessibility_label("Move up")
                    .tooltip("Move up")
                    .disabled(!self.state.controller_ready || position == 0)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: true });
                    })),
            )
            .child(
                Button::new(("queue-down", id))
                    .icon(IconName::ArrowDown)
                    .ghost()
                    .small()
                    .accessibility_label("Move down")
                    .tooltip("Move down")
                    .disabled(!self.state.controller_ready || last)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: false });
                    })),
            )
            .child(
                Button::new(("queue-remove", id))
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .accessibility_label("Remove from queue")
                    .tooltip("Remove from queue")
                    .disabled(!self.state.controller_ready)
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
                        Button::new("continue-on-error")
                            .label(if self.state.preferences.continue_on_error { "Continue on error: On" } else { "Continue on error: Off" })
                            .ghost()
                            .xsmall()
                            .disabled(!self.state.available)
                            .tooltip("Skip queued media failures only; stop after 3 consecutive failures. Saved across restarts.")
                            .on_click(cx.listener(|this, _, _, _| {
                                this.send(Command::ContinueOnError(!this.state.preferences.continue_on_error));
                            })),
                    )
                    .child(
                        Button::new("queue-clear")
                            .label("Clear queue")
                            .ghost()
                            .xsmall()
                            .disabled(!self.state.controller_ready || count == 0)
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
                        self.queue_row(position, position == last, item, cx)
                    }))
                    .into_any_element()
            })
    }
}
