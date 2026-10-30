use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    Context, ElementId, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Styled, TestSupportExt, Window, div, px,
};

use super::lists::{Tab, TabPrev};

use super::help::HelpExt;

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
        last: bool,
        item: &QueueItem,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        #[cfg(test)]
        let _row_timer = super::tests::animation_fixes::RowTimer::start();
        let theme = cx.theme();
        let id = item.id;
        // The entry that plays next is the only one with a lit position and a heavier title.
        let next = position == 0;
        let marked = next || self.added == Some(id);
        let within = self.queue_scroll.rows[position]
            .1
            .contains_focused(window, cx);
        div()
            .id(("queue-row", id))
            .w_full()
            .tab_group()
            .track_focus(&self.queue_scroll.rows[position].1)
            .tab_stop(false)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, _| {
                this.queue_key(id, position, event);
            }))
            .flex()
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
                    .text_color(if marked {
                        theme.primary
                    } else {
                        theme.muted_foreground
                    })
                    .child(format!("{:02}", position + 1)),
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
            .child(self.queue_actions(position, last, id, within, cx))
            .test_support()
    }

    /// Move up/down form one pair; Remove stands apart so it is never mistaken for a move.
    fn queue_actions(
        &self,
        position: usize,
        last: bool,
        id: u64,
        within: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        // Quiet at rest, full ink while the row holds focus; never hidden.
        let rest = if within {
            theme.muted_foreground
        } else {
            theme.muted_foreground.opacity(0.7)
        };
        let ink = |enabled: bool| {
            if enabled {
                rest
            } else {
                theme.muted_foreground.opacity(0.35)
            }
        };
        let ready = self.state.controller_ready;
        let trackers = &self.queue_scroll.actions[&ElementId::from(("queue-row", id))];
        let action = |button: Button, index: usize| {
            button
                .key_context("PlayerList")
                .on_action(cx.listener(move |this, _: &Tab, window, cx| {
                    this.list_tab(Section::Queue, position, index, false, window, cx);
                }))
                .on_action(cx.listener(move |this, _: &TabPrev, window, cx| {
                    this.list_tab(Section::Queue, position, index, true, window, cx);
                }))
                .ghost()
                .small()
        };
        let tracked = |index: usize, button: Button| {
            div()
                .track_focus(&trackers[index])
                .tab_stop(false)
                .child(button)
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .child(tracked(
                0,
                action(Button::new(("queue-up", id)), 0)
                    .icon(IconName::ArrowUp)
                    .text_color(ink(ready && position > 0))
                    .accessibility_label("Move up")
                    .help("Move up · Alt+Up", &self.help)
                    .disabled(!ready || position == 0)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: true });
                    })),
            ))
            .child(tracked(
                1,
                action(Button::new(("queue-down", id)), 1)
                    .icon(IconName::ArrowDown)
                    .text_color(ink(ready && !last))
                    .accessibility_label("Move down")
                    .help("Move down · Alt+Down", &self.help)
                    .disabled(!ready || last)
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::Shift { id, up: false });
                    })),
            ))
            .child(
                div().ml(px(8.)).child(tracked(
                    2,
                    action(Button::new(("queue-remove", id)), 2)
                        .icon(IconName::Close)
                        .text_color(ink(ready))
                        .accessibility_label("Remove from queue")
                        .help("Remove from queue · Delete", &self.help)
                        .disabled(!ready)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.send(Command::Dequeue(id));
                        })),
                )),
            )
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
