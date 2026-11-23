mod connection;
mod destination;
mod preferences;
mod reveal;
mod selectors;
use reveal::RevealExt;

pub(super) use selectors::{DestinationSelect, selectors};

use gpui_kit::component::{
    ActiveTheme, Disableable, button::ButtonVariants, scroll::ScrollableElement,
};
use gpui_kit::{
    AnyElement, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use crate::player::Command;
use crate::style::space;

use super::PlayerApp;
use super::ui::escape;

impl PlayerApp {
    /// The motion frame asks Settings which disclosure heights to sample; the
    /// reveal clock stays with the height state it owns.
    pub(super) fn sample_reveals(&mut self, window: &Window, cx: &Context<Self>) {
        if self.settings_open && !self.components.open {
            self.reveal = self.height_reveals.sample(
                self.credentials_visible(),
                self.preferences_open,
                self.setup_help,
                window,
                cx,
            );
        }
    }

    pub(super) fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.input.read(cx).focus_handle(cx).focus(window, cx);
    }

    pub(super) fn focus_settings(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.confirm_setup {
            self.confirm_setup_focus.focus(window, cx);
        } else if self.state.setup.bot.is_some() && !self.credentials_open {
            self.server_select
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        } else if !self.state.setup.external {
            self.token.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            self.settings_focus.focus(window, cx);
        }
    }

    pub(super) fn credentials_visible(&self) -> bool {
        !self.state.setup.external && (self.state.setup.bot.is_none() || self.credentials_open)
    }

    pub(super) fn settings_surface(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        self.entering_surface(div())
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if escape(&event.keystroke.key) {
                        if this.state.confirm_setup {
                            this.send(Command::ConfirmSetup(false));
                        } else if this.credentials_open {
                            this.credentials_open = false;
                            this.focus_settings(window, cx);
                        } else {
                            this.leave_settings(window, cx);
                        }
                        this.help.dismiss(cx);
                        cx.notify();
                    }
                }),
            )
            .child(self.header(cx))
            .child(self.settings(window, cx))
            .child(self.help.overlay.clone())
            .into_any_element()
    }

    pub(super) fn settings(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let setup = &self.state.setup;
        let configured = setup.bot.is_some();
        let theme = cx.theme();
        let mut panel = div()
            .id("discord-settings")
            .test_support()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .relative()
            .track_scroll(&self.settings_scroll)
            .pr_3()
            .vertical_scrollbar(&self.settings_scroll)
            .flex()
            .flex_col()
            .gap(space::SECTION)
            .text_sm()
            .line_height(px(20.))
            .track_focus(&self.settings_focus)
            .tab_stop(false)
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Discord settings"),
            );
        if self.state.confirm_setup {
            return panel.child(div().id("setup-confirmation").flex().flex_col().gap_2().role(gpui_kit::Role::Alert)
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Stop playback and switch?"))
                .child("The current track will stop and Grajek will leave voice. Your queue stays. Start playback again when you’re ready.")
                .child(div().flex().gap_2()
                    .child(gpui_kit::base::Button::new("confirm-setup").track_focus(&self.confirm_setup_focus)
                        .accessibility_label("Stop and switch").h(px(32.)).px_3().rounded(theme.radius)
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.button_primary)
                        .text_color(theme.button_primary_foreground)
                        .border_1().border_color(theme.button_primary)
                        .hover(|button| button.bg(theme.button_primary_hover))
                        .active(|button| button.bg(theme.button_primary_active))
                        .focus_visible(|button| button.border_2().border_color(theme.button_primary_foreground))
                        .child("Stop and switch")
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(true)); })))
                    .child(self.focus.button("cancel-setup").label("Keep playing")
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(false)); })))));
        }
        panel = panel.child(self.connection_settings(window, cx));
        if let Some(notice) = &self.state.notice {
            panel = panel.child(detail(notice.clone(), cx));
        }
        if configured {
            panel = panel.child(self.destination_settings(window, cx));
        }
        panel = panel.child(self.preferences_settings(cx)).child(
            div()
                .id("playback-components")
                .test_support()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Playback components"),
                )
                .child(self.component_rows(cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            self.focus
                                .button("manage-components")
                                .label(self.components_action())
                                .on_click(cx.listener(|this, _, _, cx| this.open_components(cx)))
                                .reveal("reveal-manage-components", &self.settings_scroll),
                        )
                        .child(
                            self.focus
                                .button("check-components")
                                .ghost()
                                .label("Check components")
                                .disabled(self.components_busy())
                                .on_click(cx.listener(|this, _, _, cx| this.check_components(cx)))
                                .reveal("reveal-check-components", &self.settings_scroll),
                        ),
                ),
        );
        panel
    }
}

fn detail(text: impl Into<SharedString>, cx: &Context<PlayerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
