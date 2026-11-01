use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, button::ButtonVariants};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt, Window, div,
    prelude::FluentBuilder, px,
};

use super::lists::{Tab, TabPrev};

use super::help::HelpExt;

use super::{PlayerApp, Section, clock, label};
use crate::{player::Command, store::HistoryEntry};

impl PlayerApp {
    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if self.confirm_clear {
            self.send(Command::ClearHistory);
        }
        self.confirm_clear = !self.confirm_clear;
        cx.notify();
    }

    pub(super) fn history_row(
        &self,
        index: usize,
        entry: &HistoryEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        #[cfg(test)]
        let _row_timer = super::tests::animation_fixes::RowTimer::start();
        let theme = cx.theme();
        let title = entry
            .title
            .clone()
            .unwrap_or_else(|| crate::queue::video_id(&entry.url).to_owned());
        let play_url = entry.url.clone();
        self.track_row(
            ("history-row", index).into(),
            &self.history_scroll.rows[index].1,
            window,
            cx,
        )
        .on_key_down(cx.listener(move |this, event, window, cx| {
            if this.row_menu.is_none() {
                this.list_key(Section::History, index, event, window, cx);
            }
        }))
        .child(
            self.button(("history-play", index))
                .key_context("PlayerList")
                .on_action(cx.listener(move |this, _: &Tab, window, cx| {
                    this.list_tab(Section::History, index, 0, false, window, cx);
                }))
                .on_action(cx.listener(move |this, _: &TabPrev, window, cx| {
                    this.list_tab(Section::History, index, 0, true, window, cx);
                }))
                .icon(IconName::Play)
                .ghost()
                .small()
                .accessibility_label(format!("Replay {title}. Replaces current playback"))
                .help(
                    "Replay now · replaces current playback; keeps the queue",
                    &self.help,
                )
                .disabled(!self.play_enabled())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.play_url(play_url.clone(), window, cx);
                })),
        )
        .child(label(("history-title", index), title, &self.help))
        .children(entry.duration().map(|duration| {
            div()
                .flex_none()
                .w(px(40.))
                .text_right()
                .whitespace_nowrap()
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(clock(duration))
        }))
        .child(self.row_menu(Section::History, index, index as u64, cx))
        .test_support()
    }

    pub(super) fn history_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.state.history.len();
        let confirming = self.confirm_clear && count > 0;
        div()
            .id("history-count")
            .track_focus(&self.history_header_focus)
            .tab_stop(false)
            .flex()
            .items_center()
            .gap_1()
            .child(
                self.button("history-clear")
                    .label(if confirming {
                        "Yes, clear"
                    } else {
                        "Clear history"
                    })
                    .when(confirming, ButtonVariants::danger)
                    .when(!confirming, ButtonVariants::ghost)
                    .xsmall()
                    .disabled(!self.state.controller_ready || count == 0)
                    .help(
                        "Clear all previously played tracks from History",
                        &self.help,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
            )
            .when(confirming, |row| {
                row.child(
                    self.button("history-clear-cancel")
                        .label("Cancel")
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.confirm_clear = false;
                            window.focus(&this.history_header_focus, cx);
                            cx.on_next_frame(window, |this, window, cx| {
                                if this.history_header_focus.is_focused(window) {
                                    window.focus_next(cx);
                                }
                            });
                            cx.notify();
                        })),
                )
            })
            .test_support()
    }

    pub(super) fn history_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let count = self.state.history.len();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
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
                self.scroll_list(Section::History, cx).into_any_element()
            })
    }
}
