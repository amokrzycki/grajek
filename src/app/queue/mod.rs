//! Up next: queue rows, keyboard reordering and the playback-options popover.

use std::time::Instant;

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, button::ButtonVariants, popover::Popover,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use crate::player::{Command, FAILURE_LIMIT, RepeatMode};
use crate::playlist::ImportStatus;
use crate::queue::QueueItem;

use super::fmt::label_with_tooltip;
use super::ui::menu::{Activate, menu_enter};
use super::{PlayerApp, Section};

impl PlayerApp {
    /// Alt+Up / Alt+Down move the entry holding focus; Delete removes it. Handles are keyed by
    /// entry id, so focus follows a moved entry.
    fn queue_key(&mut self, id: u64, position: usize, event: &KeyDownEvent, now: Instant) {
        let modifiers = &event.keystroke.modifiers;
        if !self.state.controller_ready {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" if modifiers.alt && position > 0 => {
                self.motion.asked_to_move(id, position, now);
                self.send(Command::Shift { id, up: true });
            }
            "down" if modifiers.alt && position + 1 < self.state.queue.len() => {
                self.motion.asked_to_move(id, position, now);
                self.send(Command::Shift { id, up: false });
            }
            "delete" if !modifiers.alt && !modifiers.control && !modifiers.shift => {
                self.send(Command::Dequeue(id));
            }
            _ => {}
        }
    }

    pub(in crate::app) fn row_wash(
        &self,
        position: usize,
        id: u64,
        window: &Window,
        cx: &gpui_kit::App,
    ) -> Option<gpui_kit::Hsla> {
        self.frame.wash(id, cx.theme().primary).filter(|_| {
            // The wash's bg would overwrite the keyboard-focus row background.
            !(self.focus.keyboard
                && self.queue_scroll.rows[position]
                    .1
                    .contains_focused(window, cx))
        })
    }

    pub(super) fn queue_row(
        &self,
        position: usize,
        item: &QueueItem,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        #[cfg(test)]
        let _row_timer = crate::app::ui::row_probe::RowTimer::start();
        let theme = cx.theme();
        let id = item.id;
        let next = position == 0;
        let wash = self.row_wash(position, id, window, cx);
        self.track_row(
            ("queue-row", id).into(),
            &self.queue_scroll.rows[position].1,
            window,
            cx,
        )
        .when_some(wash, Styled::bg)
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if this.row_menu.is_none() {
                this.list_key(Section::Queue, position, event, window, cx);
                this.queue_key(id, position, event, cx.background_executor().now());
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
                .text_color(theme.foreground)
                .child(label_with_tooltip(
                    ("queue-title", id),
                    item.label().to_owned(),
                    item.tooltip(),
                    &self.help,
                )),
        )
        .when(self.is_favorite_url(&item.url), |row| {
            row.child(Self::saved_label(
                ("queue-saved", id),
                self.saved_label_color(&item.url, cx),
            ))
        })
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
                super::ui::empty("queue-empty", "Nothing up next.", cx).into_any_element()
            } else {
                self.scroll_list(Section::Queue, cx).into_any_element()
            })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "One popover keeps playback options, queue actions and their explanations together"
    )]
    pub(super) fn queue_options(&self, cx: &Context<Self>) -> impl IntoElement {
        let repeat = self.state.preferences.repeat;
        Popover::new("queue-options-popover")
            .open(self.queue_options_open)
            .on_open_change(cx.listener(|this, open, _, cx| {
                if *open {
                    this.close_popups();
                }
                this.queue_options_open = *open;
                this.help.dismiss(cx);
                cx.notify();
            }))
            .anchor(gpui_kit::Anchor::BottomLeft)
            .trigger(
                self.focus
                    .button("queue-options")
                    .label("Playback")
                    .ghost()
                    .small()
                    .dropdown_caret(true)
                    .when(repeat != RepeatMode::Off, |button| {
                        button.text_color(cx.theme().primary)
                    })
                    .accessibility_label(format!(
                        "Queue options. {}. Remember queue, continue on error, clear queue",
                        repeat.label()
                    )),
            )
            .child(menu_enter(
                div()
                    .id("queue-options-menu")
                    .on_key_down(Self::menu_key)
                    .flex()
                    .flex_col()
                    .w(px(300.))
                    .max_h(px(392.))
                    .overflow_y_scroll()
                    .gap_1()
                    .child(self.repeat_button(cx))
                    .child(menu_detail(
                        "After a track finishes. Next and Stop never repeat.",
                        cx,
                    ))
                    .child(self.remember_button(cx))
                    .child(menu_detail(
                        "Restore pending tracks on launch. Turning off erases only the saved queue.",
                        cx,
                    ))
                    .child(
                        self.focus
                            .button("continue-on-error")
                            .key_context("PlayerMenuControl")
                            .on_action(cx.listener(|this, _: &Activate, _, _| {
                                if this.state.available {
                                    this.send(Command::ContinueOnError(
                                        !this.state.preferences.continue_on_error,
                                    ));
                                }
                            }))
                            .label(if self.state.preferences.continue_on_error {
                                "Continue on error: On"
                            } else {
                                "Continue on error: Off"
                            })
                            .child(div().flex_1())
                            .ghost()
                            .small()
                            .w_full()
                            .disabled(!self.state.available)
                            .on_click(cx.listener(|this, _, _, _| {
                                this.send(Command::ContinueOnError(
                                    !this.state.preferences.continue_on_error,
                                ));
                            })),
                    )
                    .child(menu_detail(
                        format!(
                            "Skip unplayable queued tracks. Stop after {FAILURE_LIMIT} in a row."
                        ),
                        cx,
                    ))
                    .child(
                        div()
                            .mt_2()
                            .pt_2()
                            .border_t_1()
                            .border_color(crate::style::separator())
                            .child(
                                self.focus
                                    .button("queue-clear")
                                    .key_context("PlayerMenuControl")
                                    .on_action(cx.listener(|this, _: &Activate, _, _| {
                                        if this.state.controller_ready
                                            && (!this.state.queue.is_empty()
                                                || this.state.import == ImportStatus::Importing)
                                        {
                                            this.send(Command::ClearQueue);
                                        }
                                    }))
                                    .label("Clear queue")
                                    .child(div().flex_1())
                                    .ghost()
                                    .small()
                                    .w_full()
                                    .disabled(
                                        !self.state.controller_ready
                                            || (self.state.queue.is_empty()
                                                && self.state.import != ImportStatus::Importing),
                                    )
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.send(Command::ClearQueue);
                                    })),
                            ),
                    )
                    .child(menu_detail(
                        "Keeps the current track; cancels playlist imports.",
                        cx,
                    ))
                    .child(
                        div()
                            .pt_2()
                            .px_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Preferences save automatically."),
                    )
                    .test_support(),
                "queue-options-menu-enter",
                cx,
            ))
    }

    pub(super) fn repeat_button(&self, cx: &Context<Self>) -> impl IntoElement {
        self.focus
            .button("repeat")
            .key_context("PlayerMenuControl")
            .on_action(cx.listener(|this, _: &Activate, _, _| {
                if this.state.available {
                    this.send(Command::Repeat(this.state.preferences.repeat.next()));
                }
            }))
            .label(self.state.preferences.repeat.label())
            .child(div().flex_1())
            .accessibility_label(self.state.preferences.repeat.label())
            .ghost()
            .small()
            .w_full()
            .disabled(!self.state.available)
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::Repeat(this.state.preferences.repeat.next()));
            }))
    }

    pub(super) fn remember_button(&self, cx: &Context<Self>) -> impl IntoElement {
        self.focus
            .button("remember-queue")
            .key_context("PlayerMenuControl")
            .on_action(cx.listener(|this, _: &Activate, _, _| {
                if this.state.available {
                    this.send(Command::RememberQueue(
                        !this.state.preferences.remember_queue,
                    ));
                }
            }))
            .label(if self.state.preferences.remember_queue {
                "Remember queue: On"
            } else {
                "Remember queue: Off"
            })
            .child(div().flex_1())
            .ghost()
            .small()
            .w_full()
            .disabled(!self.state.available)
            .on_click(cx.listener(|this, _, _, _| {
                this.send(Command::RememberQueue(
                    !this.state.preferences.remember_queue,
                ));
            }))
    }
}

fn menu_detail(text: impl IntoElement, cx: &Context<PlayerApp>) -> impl IntoElement {
    div()
        .px_2()
        .pb_1()
        .text_xs()
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
}
