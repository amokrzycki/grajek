mod reveal;
use reveal::RevealExt;

use gpui_kit::component::{
    ActiveTheme, IndexPath, Sizable,
    button::ButtonVariants,
    scroll::ScrollableElement,
    select::{Select, SelectDelegate, SelectEvent, SelectItem, SelectState},
    switch::Switch,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{AnyElement, App, Focusable, SharedString, TestSupportExt, px};

use super::{
    AppContext, Button, Context, Disableable, Entity, FontWeight, Input, InputState,
    InteractiveElement, IntoElement, ParentElement, PlayerApp, StatefulInteractiveElement, Styled,
    Window, div,
};
use crate::player::Command;
use crate::setup::{Destination, Request, Status};
use crate::style::space;

#[derive(Clone)]
pub(super) struct Choice {
    id: u64,
    name: String,
    reason: Option<String>,
}

impl SelectItem for Choice {
    type Value = u64;
    fn title(&self) -> SharedString {
        self.name.clone().into()
    }
    fn value(&self) -> &u64 {
        &self.id
    }
    fn disabled(&self) -> bool {
        self.reason.is_some()
    }
}

pub(super) struct Choices(Vec<Choice>);
impl SelectDelegate for Choices {
    type Item = Choice;
    fn items_count(&self, _: usize) -> usize {
        self.0.len()
    }
    fn item(&self, ix: IndexPath) -> Option<&Choice> {
        self.0.get(ix.row)
    }
    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Choice: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.0
            .iter()
            .position(|item| item.value() == value)
            .map(IndexPath::new)
    }
    fn on_will_change(
        &mut self,
        selection: &mut Vec<(IndexPath, Choice)>,
        changes: &[gpui_kit::component::searchable_list::SearchableListChange],
    ) {
        use gpui_kit::component::searchable_list::SearchableListChange;
        // Kit prevents pointer activation, but keyboard cursors can inspect disabled rows.
        if changes.iter().any(|change| matches!(change, SearchableListChange::Select { index } if self.item(*index).is_some_and(SelectItem::disabled))) {
            return;
        }
        selection.clear();
        for change in changes {
            if let SearchableListChange::Select { index } = change
                && let Some(item) = self.item(*index)
            {
                selection.push((*index, item.clone()));
            }
        }
    }
    fn render_item(
        &self,
        _: IndexPath,
        item: &Choice,
        checked: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let reason = item.reason.as_ref().map(|reason| {
            if reason.contains("View Channel") {
                "Needs View Channel, Connect and Speak"
            } else {
                reason.as_str()
            }
        });
        Some(
            div()
                .id(("destination-choice", item.id))
                .w_full()
                .h(px(48.))
                .flex()
                .items_center()
                .gap_2()
                .role(gpui_kit::Role::ListBoxOption)
                .aria_selected(checked)
                .a11y_synthetic_children({
                    let disabled = item.disabled();
                    move |builder| {
                        if disabled {
                            builder.parent_node().set_disabled();
                        }
                    }
                })
                .when(item.disabled(), |row| {
                    row.on_click(|_, _, cx| cx.stop_propagation())
                })
                .aria_label(format!(
                    "{}{}",
                    item.name,
                    item.reason
                        .as_ref()
                        .map_or(String::new(), |reason| format!(". Unavailable: {reason}"))
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .line_height(px(18.))
                                .when(item.disabled(), Styled::truncate)
                                .when(!item.disabled(), |name| name.line_clamp(2))
                                .child(item.name.clone()),
                        )
                        .children(reason.map(|reason| {
                            div()
                                .id(("destination-reason", item.id))
                                .text_xs()
                                .line_height(px(16.))
                                .text_color(cx.theme().muted_foreground)
                                .child(reason.to_owned())
                                .test_support()
                        })),
                )
                .children(checked.then(|| {
                    gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Check)
                        .small()
                        .text_color(cx.theme().primary)
                }))
                .test_support()
                .into_any_element(),
        )
    }
}

pub(super) type DestinationSelect = Entity<SelectState<Choices>>;

pub(super) fn selectors(
    window: &mut Window,
    cx: &mut App,
) -> (DestinationSelect, DestinationSelect) {
    (
        cx.new(|cx| SelectState::new(Choices(Vec::new()), None, window, cx)),
        cx.new(|cx| SelectState::new(Choices(Vec::new()), None, window, cx)),
    )
}

impl PlayerApp {
    pub(super) fn subscribe_selectors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.push(cx.subscribe_in(
            &self.server_select,
            window,
            |this, _, event, _, _| {
                if let SelectEvent::Confirm(Some(id)) = event
                    && !this.state.setup_busy
                    && this
                        .state
                        .setup
                        .servers
                        .iter()
                        .any(|server| server.id.get() == *id && server.available)
                    && this
                        .state
                        .setup
                        .browsing
                        .is_none_or(|guild| guild.get() != *id)
                {
                    this.send(Command::Setup(Request::Channels(
                        serenity::all::GuildId::new(*id),
                    )));
                }
            },
        ));
        self.subscriptions.push(cx.subscribe_in(
            &self.channel_select,
            window,
            |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(id)) = event
                    && !this.state.setup_busy
                    && let (Some(bot), Some(guild)) =
                        (this.state.setup.bot, this.state.setup.browsing)
                    && this
                        .state
                        .setup
                        .channels
                        .iter()
                        .any(|channel| channel.id.get() == *id && channel.reason.is_none())
                    && this
                        .state
                        .setup
                        .destination
                        .as_ref()
                        .is_none_or(|destination| {
                            destination.guild != guild || destination.channel.get() != *id
                        })
                {
                    this.send(Command::Setup(Request::Select(Destination {
                        bot,
                        guild,
                        channel: serenity::all::ChannelId::new(*id),
                        guild_name: String::new(),
                        channel_name: String::new(),
                    })));
                    // Select commits a cursor locally; only the controller may commit the destination.
                    this.sync_selectors(window, cx);
                }
            },
        ));
        self.sync_selectors(window, cx);
    }

    pub(super) fn sync_selectors(&self, window: &mut Window, cx: &mut Context<Self>) {
        let setup = &self.state.setup;
        let servers = Choices(
            setup
                .servers
                .iter()
                .map(|server| Choice {
                    id: server.id.get(),
                    name: server.name.clone(),
                    reason: (!server.available)
                        .then(|| "Temporarily unavailable · refresh to retry".into()),
                })
                .collect(),
        );
        let channels = Choices(
            setup
                .channels
                .iter()
                .map(|channel| Choice {
                    id: channel.id.get(),
                    name: channel.name.clone(),
                    reason: channel.reason.clone(),
                })
                .collect(),
        );
        self.server_select.update(cx, |select, cx| {
            select.set_items(servers, window, cx);
            let selected = setup.browsing.or_else(|| {
                setup
                    .destination
                    .as_ref()
                    .map(|destination| destination.guild)
            });
            if let Some(id) = selected {
                select.set_selected_value(&id.get(), window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
        self.channel_select.update(cx, |select, cx| {
            select.set_items(channels, window, cx);
            let selected = setup
                .destination
                .as_ref()
                .filter(|destination| Some(destination.guild) == setup.browsing);
            if let Some(destination) = selected {
                select.set_selected_value(&destination.channel.get(), window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
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

    pub(super) fn settings(&self, cx: &Context<Self>) -> impl IntoElement {
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
                        .flex().items_center().justify_center().bg(theme.button_primary).text_color(theme.button_primary_foreground)
                        .border_1().border_color(theme.button_primary)
                        .hover(|button| button.bg(theme.button_primary_hover))
                        .active(|button| button.bg(theme.button_primary_active))
                        .focus(|button| button.border_2().border_color(theme.button_primary_foreground))
                        .child("Stop and switch")
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(true)); })))
                    .child(Button::new("cancel-setup").label("Keep playing")
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::ConfirmSetup(false)); })))));
        }
        panel = panel.child(self.connection_settings(cx));
        if let Some(notice) = &self.state.notice {
            panel = panel.child(detail(notice.clone(), cx));
        }
        if configured {
            panel = panel.child(self.destination_settings(cx));
        }
        panel.child(self.preferences_settings(cx))
    }

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

    fn connection_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        let configured = setup.bot.is_some();
        let credentials = !configured || self.credentials_open;
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
                        Button::new("change-credentials")
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
                        Button::new("retry-authentication")
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
        } else if credentials {
            connection = connection.child(self.credential_settings(cx));
        }
        connection
    }

    fn credential_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.state.setup_busy;
        div().flex().flex_none().flex_col().gap_2()
            .when(self.state.setup.bot.is_none(), |panel| panel.child(detail("Play YouTube in a Discord voice channel through your own bot.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child("Bot token")
                .child(Input::new(&self.token).id("bot-token").aria_label("Discord bot token").disabled(busy).reveal("token-control", &self.settings_scroll)))
            .when(self.token_error, |panel| panel.child(div().id("token-error").role(gpui_kit::Role::Alert).text_xs().text_color(cx.theme().danger).child("Paste a bot token before connecting.").test_support()))
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("save-token").primary().label("Connect and remember").disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.authenticate(true, window, cx))).reveal("save-control", &self.settings_scroll))
                .child(Button::new("session-token").label("Use for this session").disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.authenticate(false, window, cx))).reveal("session-control", &self.settings_scroll)))
            .child(detail("Remember stores the token in your OS credential vault. Session-only lasts until Quit; an older saved token may return next launch. Never use a personal Discord token or password.", cx))
            .child(div().flex().child(Button::new("setup-help").ghost().small().label(if self.setup_help { "Hide bot setup guide" } else { "Create and invite a bot…" })
                .on_click(cx.listener(|this, _, _, cx| { this.setup_help = !this.setup_help; cx.notify(); })).reveal("guide-control", &self.settings_scroll)))
            .when(self.setup_help, |panel| panel.child(div().flex().flex_col().gap_2()
                .child(detail("In the Discord Developer Portal, create an application. Open Bot and copy its bot token here. Keep it private: anyone with it can control your bot.", cx))
                .child(detail("Under OAuth2 → URL Generator, choose bot with View Channels, Connect and Speak. Open the generated link to invite the bot to your server. No privileged intents are needed.", cx))
                .child(Button::new("developer-portal").label("Open Developer Portal").on_click(|_, _, cx| cx.open_url("https://discord.com/developers/applications")).reveal("portal-control", &self.settings_scroll))))
            .when(self.state.setup.bot.is_some() || matches!(self.state.setup.status, Status::Failed(_)) || self.state.setup.storage_notice.is_some(), |panel| panel
                .child(div().flex().child(Button::new("remove-token").ghost().label("Remove credentials and disconnect").disabled(busy)
                    .on_click(cx.listener(|this, _, _, _| { this.send(Command::Setup(Request::Remove)); })).reveal("remove-control", &self.settings_scroll)))
                .child(detail("Also removes the saved token from this computer. It does not revoke the token on Discord.", cx)))
    }

    fn destination_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        let mut panel = div().flex().flex_none().flex_col().gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Playback destination"))
            .when(setup.destination.is_none(), |panel| panel.child(detail("Choose a server, then a voice channel. Selecting a channel won’t join voice or start music.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child(div().flex().items_center().justify_between().child("Server")
                    .child(Button::new("refresh-servers").ghost().small().label("Refresh").accessibility_label("Refresh servers").disabled(busy)
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::Setup(Request::Refresh)); })).reveal("refresh-server-control", &self.settings_scroll)))
                .child(Select::new(&self.server_select).id("server-selector").w_full().h(px(32.)).accessibility_label("Discord server")
                    .placeholder("Choose a server").menu_max_h(px(224.)).disabled(busy || setup.servers.is_empty()).reveal("server-control", &self.settings_scroll)))
            .when(setup.servers.is_empty() && !busy, |panel| panel.child(detail("No servers found. Invite this bot to your server, then refresh.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child(div().flex().items_center().justify_between().child("Voice channel")
                    .child(Button::new("refresh-channels").ghost().small().label("Refresh").accessibility_label("Refresh voice channels")
                        .disabled(busy || setup.browsing.is_none())
                        .on_click(cx.listener(|this, _, _, _| { if let Some(guild) = this.state.setup.browsing { this.send(Command::Setup(Request::Channels(guild))); } })).reveal("refresh-channel-control", &self.settings_scroll)))
                .child(Select::new(&self.channel_select).id("channel-selector").w_full().h(px(32.)).accessibility_label("Discord voice channel")
                    .placeholder(if setup.browsing.is_none() { "Choose a server first" } else { "Choose a voice channel" })
                    .menu_max_h(px(224.)).disabled(busy || setup.browsing.is_none() || setup.channels.is_empty()).reveal("channel-control", &self.settings_scroll)))
            .when(setup.browsing.is_some() && setup.channels.is_empty() && !busy, |panel| panel.child(detail("No voice channels found. Check the bot’s access in this server, then refresh.", cx)));
        if setup
            .channels
            .iter()
            .any(|channel| channel.reason.is_some())
        {
            panel = panel.child(detail("Unavailable channels explain why in the list. Ask a server administrator to update the bot’s channel permissions.", cx));
        }
        if let Some(destination) = &setup.destination {
            panel = panel.child(
                div()
                    .id("active-destination")
                    .role(gpui_kit::Role::Status)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Selected: {} / {}",
                        destination.guild_name, destination.channel_name
                    ))
                    .test_support(),
            );
            if self.state.available {
                panel = panel.child(
                    div().flex().child(
                        Button::new("done-settings")
                            .primary()
                            .label("Back to music")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.settings_open = false;
                                this.help.dismiss(cx);
                                this.input.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            })),
                    ),
                );
            }
        }
        panel
    }

    fn preferences_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        div().flex().flex_none().flex_col().gap_2()
            .child(div().flex().child(Button::new("preferences-disclosure").ghost().small().label(if self.preferences_open { "Hide preferences" } else { "Preferences…" })
                .on_click(cx.listener(|this, _, _, cx| { this.preferences_open = !this.preferences_open; cx.notify(); })).reveal("preferences-control", &self.settings_scroll)))
            .when(self.preferences_open, |panel| panel
                .child(Switch::new("close-to-tray").label("Keep playing when the window closes").checked(self.state.close_to_tray)
                    .on_change(cx.listener(|this, enabled, _, _| { this.send(Command::CloseToTray(*enabled)); })).reveal("tray-control", &self.settings_scroll))
                .child(detail("Uses the system tray when available. Choose Quit in the tray to stop Grajek. Without a tray, closing the window quits.", cx)))
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

fn detail(text: impl Into<SharedString>, cx: &Context<PlayerApp>) -> impl IntoElement {
    div()
        .text_xs()
        .line_height(px(16.))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
