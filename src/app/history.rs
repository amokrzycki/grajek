use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt, div,
    prelude::FluentBuilder, px,
};

use super::{PlayerApp, clock, label};
use crate::{player::Command, store::HistoryEntry};

impl PlayerApp {
    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.confirm_clear {
            self.send(Command::ClearHistory);
        }
        self.confirm_clear = !self.confirm_clear;
        cx.notify();
    }

    fn history_row(
        &self,
        index: usize,
        entry: &HistoryEntry,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let title = entry
            .title
            .clone()
            .unwrap_or_else(|| crate::queue::video_id(&entry.url).to_owned());
        let url = entry.url.clone();
        let (play_url, queue_url, forget_url) = (url.clone(), url.clone(), url);
        div()
            .id(("history-row", index))
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(36.))
            .rounded(theme.radius)
            .text_sm()
            .hover(|row| row.bg(theme.list_hover))
            .child(label(("history-title", index), title))
            .children(entry.duration().map(|duration| {
                div()
                    .flex_none()
                    .w(px(48.))
                    .text_right()
                    .whitespace_nowrap()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(clock(duration))
            }))
            .child(
                Button::new(("history-play", index))
                    .icon(IconName::Play)
                    .ghost()
                    .small()
                    .accessibility_label("Play now")
                    .tooltip("Play now")
                    .disabled(!self.state.can_play())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.play_url(play_url.clone(), window, cx);
                    })),
            )
            .child(
                Button::new(("history-queue", index))
                    .icon(IconName::Plus)
                    .ghost()
                    .small()
                    .accessibility_label("Add to queue")
                    .tooltip("Add to queue")
                    .disabled(!self.state.can_enqueue())
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.enqueue_url(queue_url.clone());
                    })),
            )
            .child(
                Button::new(("history-remove", index))
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .accessibility_label("Remove from history")
                    .tooltip("Remove from history")
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.send(Command::ForgetHistory(forget_url.clone()));
                    })),
            )
            .test_support()
    }

    pub(super) fn history_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let count = self.state.history.len();
        let confirming = self.confirm_clear && count > 0;
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
                    .gap_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .id("history-count")
                            .child(format!("{count} saved"))
                            .test_support(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .when(confirming, |row| {
                                row.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .text_color(theme.foreground)
                                        .child(format!("Clear all {count}?")),
                                )
                            })
                            .child(
                                Button::new("history-clear")
                                    .label(if confirming {
                                        "Yes, clear"
                                    } else {
                                        "Clear history"
                                    })
                                    .when(confirming, ButtonVariants::danger)
                                    .when(!confirming, ButtonVariants::ghost)
                                    .xsmall()
                                    .disabled(count == 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                            )
                            .when(confirming, |row| {
                                row.child(
                                    Button::new("history-clear-cancel")
                                        .label("Cancel")
                                        .ghost()
                                        .xsmall()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_clear = false;
                                            cx.notify();
                                        })),
                                )
                            }),
                    ),
            )
            .children(self.state.notice.clone().map(|notice| {
                div()
                    .id("notice")
                    .text_xs()
                    .text_color(theme.yellow)
                    .line_clamp(2)
                    .child(notice)
                    .test_support()
            }))
            .child(if count == 0 {
                Self::empty(
                    "history-empty",
                    "Nothing played yet. Videos appear here once they start playing.",
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .id("history-list")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scrollbar()
                    .children(
                        self.state
                            .history
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| self.history_row(index, entry, cx)),
                    )
                    .into_any_element()
            })
    }
}
