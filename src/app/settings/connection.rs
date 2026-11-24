use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::ButtonVariants,
    input::{Input, InputState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div,
};

use crate::discord::setup::{Request, Status};
use crate::player::Command;

use super::PlayerApp;
use super::detail;
use super::reveal::RevealExt;

impl PlayerApp {
    fn setup_status(&self) -> &'static str {
        if self.state.setup_busy {
            "Connecting or updating Discord…"
        } else {
            match self.state.setup.status {
                Status::NeedsToken => "Connect a bot to get started",
                Status::Checking => "Checking bot credentials…",
                Status::NeedsDestination | Status::Ready => "Bot connected",
                Status::Failed(_) if self.state.setup.bot.is_some() => "Discord needs attention",
                Status::Failed(_) => "Couldn’t connect the bot",
            }
        }
    }

    pub(super) fn connection_settings(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        let configured = setup.bot.is_some();
        let theme = cx.theme();
        let status = self.setup_status();
        let mut connection = div().flex().flex_none().flex_col().gap_2().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .id("connection-status")
                        .role(gpui_kit::Role::Status)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            if configured || busy || matches!(setup.status, Status::Checking) {
                                status
                            } else {
                                "Connect your Discord bot"
                            },
                        ),
                )
                .when(configured && !setup.external, |row| {
                    row.child(
                        self.focus
                            .button("change-credentials")
                            .ghost()
                            .small()
                            .label(if self.credentials_open {
                                "Cancel changes"
                            } else {
                                "Change credentials"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.credentials_open = !this.credentials_open;
                                if this.credentials_open {
                                    this.token.read(cx).focus_handle(cx).focus(window, cx);
                                } else {
                                    this.token.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                    });
                                    this.token_error = false;
                                    this.server_select
                                        .read(cx)
                                        .focus_handle(cx)
                                        .focus(window, cx);
                                }
                                cx.notify();
                            }))
                            .reveal("change-control", &self.settings_scroll),
                    )
                }),
        );
        if let Status::Failed(message) = &setup.status {
            connection = connection
                .child(
                    div()
                        .id("connection-error")
                        .role(gpui_kit::Role::Alert)
                        .text_color(theme.danger)
                        .child(message.clone()),
                )
                .child(
                    div().flex().child(
                        self.focus
                            .button("retry-authentication")
                            .label("Retry connection")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, _| {
                                this.send(Command::Setup(Request::Retry));
                            }))
                            .reveal("retry-control", &self.settings_scroll),
                    ),
                );
        }
        if let Some(notice) = &setup.storage_notice {
            connection = connection.child(
                div()
                    .id("storage-notice")
                    .role(gpui_kit::Role::Alert)
                    .text_xs()
                    .text_color(theme.danger)
                    .child(notice.clone()),
            );
        }
        if setup.external {
            connection = connection.child(detail("Bot credentials come from your environment or .env file. Change or remove them there.", cx));
        } else {
            connection = connection.children(self.reveal.panel(
                "credentials-reveal",
                self.reveal.credentials,
                self.credentials_visible(),
                || self.credential_settings(window, cx).into_any_element(),
            ));
        }
        connection
    }

    fn credential_settings(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.state.setup_busy;
        div().flex().flex_none().flex_col().gap_2()
            .when(self.state.setup.bot.is_none(), |panel| panel.child(detail("Play YouTube in a Discord voice channel through your own bot.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child("Bot token")
                .child(Input::new(&self.token).focus_bordered(self.focus.keyboard || self.token_error).border_color(self.focus.editing_border(self.token.read(cx).focus_handle(cx).is_focused(window), cx)).id("bot-token").aria_label("Discord bot token").disabled(busy).reveal("token-control", &self.settings_scroll)))
            .when(self.token_error, |panel| panel.child(div().id("token-error").role(gpui_kit::Role::Alert).text_xs().text_color(cx.theme().danger).child("Paste a bot token before connecting.").test_support()))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.focus.button("save-token").primary().label("Connect and remember").disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.authenticate(true, window, cx))).reveal("save-control", &self.settings_scroll))
                .child(self.focus.button("session-token").label("Use for this session").disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.authenticate(false, window, cx))).reveal("session-control", &self.settings_scroll)))
            .child(detail("Remember stores the token in your OS credential vault. Session-only lasts until Quit; an older saved token may return next launch. Never use a personal Discord token or password.", cx))
            .child(div().flex().child(self.focus.button("setup-help").ghost().small().label(if self.setup_help { "Hide bot setup guide" } else { "Create and invite a bot…" })
                .on_click(cx.listener(|this, _, _, cx| { this.setup_help = !this.setup_help; cx.notify(); })).reveal("guide-control", &self.settings_scroll)))
            .children(self.reveal.panel("guide-reveal", self.reveal.guide, self.setup_help, || {
                div().flex().flex_col().gap_2()
                    .child(detail("In the Discord Developer Portal, create an application. Open Bot and copy its bot token here. Keep it private: anyone with it can control your bot.", cx))
                    .child(detail("Under OAuth2 → URL Generator, choose bot with View Channels, Connect and Speak. Open the generated link to invite the bot to your server. No privileged intents are needed.", cx))
                    .child(self.focus.button("developer-portal").label("Open Developer Portal").on_click(|_, _, cx| cx.open_url("https://discord.com/developers/applications")).reveal("portal-control", &self.settings_scroll))
                    .into_any_element()
            }))
            .when(
                self.state.setup.bot.is_some()
                    || matches!(self.state.setup.status, Status::Failed(_))
                    || self.state.setup.storage_notice.is_some(),
                |panel| {
                    panel
                        .child(
                            div().flex().child(
                                self.focus
                                    .button("remove-token")
                                    .ghost()
                                    .label("Remove credentials and disconnect")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.send(Command::Setup(Request::Remove));
                                    }))
                                    .reveal("remove-control", &self.settings_scroll),
                            ),
                        )
                        .child(detail(
                            "Also removes the saved token from this computer. It does not revoke the token on Discord.",
                            cx,
                        ))
                },
            )
    }

    fn authenticate(&mut self, remember: bool, window: &mut Window, cx: &mut Context<Self>) {
        let token = self.token.read(cx).value().to_string();
        if token.trim().is_empty() {
            self.token_error = true;
            self.token.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
            return;
        }
        self.token_error = false;
        if self.send(Command::Setup(Request::Authenticate { token, remember })) {
            self.token = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Paste bot token")
            });
            self.token.read(cx).focus_handle(cx).focus(window, cx);
        }
    }
}
