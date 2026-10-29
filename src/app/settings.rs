use super::{
    AppContext, Button, Command, Context, Disableable, FontWeight, Input, InputState,
    InteractiveElement, IntoElement, ParentElement, PlayerApp, StatefulInteractiveElement, Styled,
    Window, div,
};
use crate::setup::{Destination, Request, Status};

impl PlayerApp {
    pub(super) fn settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        let status = if busy {
            "Working…".into()
        } else {
            match &setup.status {
                Status::NeedsToken => "Connect your Discord bot".into(),
                Status::Checking => "Checking bot token…".into(),
                Status::NeedsDestination => {
                    "Bot authenticated · choose a server and voice channel".into()
                }
                Status::Ready => "Ready · playback starts only when you ask".into(),
                Status::Failed(message) => message.clone(),
            }
        };
        let mut panel = div()
            .id("discord-settings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .text_sm()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Discord settings"),
            )
            .child(status);
        if matches!(setup.status, Status::Failed(_)) {
            panel = panel.child(
                Button::new("retry-authentication")
                    .label("Retry connection")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, _| {
                        this.send(Command::Setup(Request::Retry));
                    })),
            );
        }
        if let Some(notice) = &self.state.notice {
            panel = panel.child(notice.clone());
        }
        if let Some(notice) = &setup.storage_notice {
            panel = panel.child(notice.clone());
        }
        if self.state.confirm_setup {
            return panel.child("Switching stops the current track and leaves voice. Your pending queue stays. Playback will not restart automatically.")
                .child(div().flex().gap_2()
                    .child(Button::new("confirm-setup").label("Stop and switch").on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(true)); })))
                    .child(Button::new("cancel-setup").label("Keep playing").on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(false)); }))));
        }
        panel = panel.child("Grajek connects as your bot to play audio. Use a bot token, never a personal user token or Discord password.");
        if setup.external {
            panel = panel.child(
                "Token managed by your environment / .env. Grajek will not replace or remove it.",
            );
        } else {
            panel = panel.child(Input::new(&self.token).id("bot-token").aria_label("Discord bot token").disabled(busy))
                .child(div().flex().flex_wrap().gap_2()
                    .child(Button::new("save-token").label("Connect and remember").disabled(busy).on_click(cx.listener(|this, _, window, cx| this.authenticate(true, window, cx))))
                    .child(Button::new("session-token").label("Use for this session").disabled(busy).on_click(cx.listener(|this, _, window, cx| this.authenticate(false, window, cx))))
                    .child(Button::new("remove-token").label("Remove saved token").disabled(busy).on_click(cx.listener(|this, _, _, _| { this.send(Command::Setup(Request::Remove)); }))))
                .child("Remember uses secure OS storage. Session-only keeps this token until Quit; an older saved token may return next launch. Removing it here does not revoke it on Discord.");
        }
        panel = panel.child(
            Button::new("setup-help")
                .label(if self.setup_help {
                    "Hide setup help"
                } else {
                    "How to create and invite a bot"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.setup_help = !this.setup_help;
                    cx.notify();
                })),
        );
        if self.setup_help {
            panel = panel.child("Create an application in the Developer Portal. Under Bot, copy its token. Under OAuth2 → URL Generator, select bot and View Channels, Connect, Speak; open that link to invite it. No privileged intents are needed.")
                .child(Button::new("developer-portal").label("Open Discord Developer Portal").on_click(|_, _, cx| cx.open_url("https://discord.com/developers/applications")));
        }
        self.tray_settings(self.destination_settings(panel, cx), cx)
    }

    fn tray_settings(
        &self,
        panel: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let enabled = self.state.close_to_tray;
        panel.child(div().font_weight(FontWeight::SEMIBOLD).child("Background playback"))
            .child(Button::new("close-to-tray").label(if enabled { "Close to tray: On" } else { "Close to tray: Off" })
                .on_click(cx.listener(move |this, _, _, _| { this.send(Command::CloseToTray(!enabled)); })))
            .child("When enabled, Close minimizes Grajek while audio continues. Quit in the tray stops the bot. Without a tray, Close quits.")
    }

    fn destination_settings(
        &self,
        mut panel: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        if let Some(bot) = setup.bot {
            panel = panel.child(
                Button::new("refresh-servers")
                    .label("Refresh servers")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, _| {
                        this.send(Command::Setup(Request::Refresh));
                    })),
            );
            if setup.servers.is_empty() {
                panel = panel.child("No servers yet. Invite this bot to a server, then refresh.");
            }
            for server in &setup.servers {
                let id = server.id;
                panel = panel.child(
                    Button::new(("server", id.get()))
                        .label(server.name.clone())
                        .tooltip(if server.available {
                            server.name.clone()
                        } else {
                            "Server temporarily unavailable. Refresh the server list to retry."
                                .into()
                        })
                        .disabled(busy || !server.available)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.send(Command::Setup(Request::Channels(id)));
                        })),
                );
            }
            if let Some(guild) = setup.browsing {
                panel = panel
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Voice channel"),
                    )
                    .child(
                        Button::new("refresh-channels")
                            .label("Refresh channels")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.send(Command::Setup(Request::Channels(guild)));
                            })),
                    );
                if setup.channels.is_empty() && !busy {
                    panel = panel.child("No voice channels available. Check the bot’s server permissions, then refresh.");
                }
                for channel in &setup.channels {
                    let channel_id = channel.id;
                    let selected = setup
                        .destination
                        .as_ref()
                        .is_some_and(|destination| destination.channel == channel.id);
                    let name = format!(
                        "{}{}",
                        channel.name,
                        if selected { " · selected" } else { "" }
                    );
                    panel = panel.child(
                        Button::new(("channel", channel.id.get()))
                            .label(name)
                            .tooltip(
                                channel
                                    .reason
                                    .clone()
                                    .unwrap_or_else(|| channel.name.clone()),
                            )
                            .disabled(busy || channel.reason.is_some())
                            .on_click(cx.listener(move |this, _, _, _| {
                                this.send(Command::Setup(Request::Select(Destination {
                                    bot,
                                    guild,
                                    channel: channel_id,
                                    guild_name: String::new(),
                                    channel_name: String::new(),
                                })));
                            })),
                    );
                    if let Some(reason) = &channel.reason {
                        panel = panel.child(reason.clone());
                    }
                }
            }
        }
        panel
    }

    fn authenticate(&mut self, remember: bool, window: &mut Window, cx: &mut Context<Self>) {
        let token = self.token.read(cx).value().to_string();
        if token.trim().is_empty() {
            return;
        }
        if self.send(Command::Setup(Request::Authenticate { token, remember })) {
            self.token = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("Bot token")
            });
        }
    }
}
