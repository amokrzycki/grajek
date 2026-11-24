use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, button::ButtonVariants, select::Select,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use crate::discord::setup::Request;
use crate::player::Command;

use super::PlayerApp;
use super::detail;
use super::reveal::RevealExt;

impl PlayerApp {
    pub(super) fn destination_settings(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let setup = &self.state.setup;
        let busy = self.state.setup_busy;
        let mut panel = div().flex().flex_none().flex_col().gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Playback destination"))
            .when(setup.destination.is_none(), |panel| panel.child(detail("Choose a server, then a voice channel. Selecting a channel won’t join voice or start music.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child(div().flex().items_center().justify_between().child("Server")
                    .child(self.focus.button("refresh-servers").ghost().small().label("Refresh").accessibility_label("Refresh servers").disabled(busy)
                        .on_click(cx.listener(|this, _, _, _| { this.send(Command::Setup(Request::Refresh)); })).reveal("refresh-server-control", &self.settings_scroll)))
                .child(Select::new(&self.server_select).appearance(false)
                    .bg(cx.theme().muted).text_color(cx.theme().foreground).rounded(cx.theme().radius)
                    .border_1().border_color(self.focus.editing_border(self.server_select.read(cx).focus_handle(cx).is_focused(window), cx)).id("server-selector").w_full().h(px(32.)).accessibility_label("Discord server")
                    .placeholder("Choose a server").menu_max_h(px(224.)).when(busy || setup.servers.is_empty(), |select| select.opacity(0.5)).disabled(busy || setup.servers.is_empty()).reveal("server-control", &self.settings_scroll)))
            .when(setup.servers.is_empty() && !busy, |panel| panel.child(detail("No servers found. Invite this bot to your server, then refresh.", cx)))
            .child(div().flex().flex_col().gap_1()
                .child(div().flex().items_center().justify_between().child("Voice channel")
                    .child(self.focus.button("refresh-channels").ghost().small().label("Refresh").accessibility_label("Refresh voice channels")
                        .disabled(busy || setup.browsing.is_none())
                        .on_click(cx.listener(|this, _, _, _| { if let Some(guild) = this.state.setup.browsing { this.send(Command::Setup(Request::Channels(guild))); } })).reveal("refresh-channel-control", &self.settings_scroll)))
                .child(Select::new(&self.channel_select).appearance(false)
                    .bg(cx.theme().muted).text_color(cx.theme().foreground).rounded(cx.theme().radius)
                    .border_1().border_color(self.focus.editing_border(self.channel_select.read(cx).focus_handle(cx).is_focused(window), cx)).id("channel-selector").w_full().h(px(32.)).accessibility_label("Discord voice channel")
                    .placeholder(if setup.browsing.is_none() { "Choose a server first" } else { "Choose a voice channel" })
                    .menu_max_h(px(224.)).when(busy || setup.browsing.is_none() || setup.channels.is_empty(), |select| select.opacity(0.5)).disabled(busy || setup.browsing.is_none() || setup.channels.is_empty()).reveal("channel-control", &self.settings_scroll)))
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
                        self.focus
                            .button("done-settings")
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
}
