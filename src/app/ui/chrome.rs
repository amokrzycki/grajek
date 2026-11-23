use gpui_kit::assets::IconName as AudioIcon;
use gpui_kit::component::{ActiveTheme, Icon, button::ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px, relative,
};

use crate::player::{Phase, State};
use crate::style::space;

use super::super::fmt::{format_progress, heading_size, label};
use super::super::transport::Pending;
use super::super::{PlayerApp, motion};
use super::help::HelpExt;

fn state_symbol(phase: Phase) -> AudioIcon {
    match phase {
        Phase::Playing => AudioIcon::Play,
        Phase::Paused => AudioIcon::Pause,
        Phase::Ready | Phase::Error => AudioIcon::Square,
        Phase::Initializing
        | Phase::Connecting
        | Phase::Resolving
        | Phase::Stopping
        | Phase::Disconnecting => AudioIcon::Ellipsis,
    }
}

fn connection(state: &State) -> &'static str {
    if state.setup.bot.is_none()
        && matches!(
            state.setup.status,
            crate::discord::setup::Status::NeedsToken
        )
        && !state.available
    {
        "Bot setup needed"
    } else if matches!(
        state.setup.status,
        crate::discord::setup::Status::NeedsDestination
    ) {
        "Choose a voice channel"
    } else if state.phase == Phase::Disconnecting {
        "Leaving Discord voice…"
    } else if state.available && state.voice_connected {
        "Voice connected"
    } else if state.available {
        "Voice idle"
    } else if state.phase == Phase::Initializing {
        "Connecting to Discord…"
    } else {
        "Discord unavailable"
    }
}

fn connection_color(state: &State, theme: &gpui_kit::component::Theme) -> gpui_kit::Hsla {
    match (&state.setup.status, state.phase, state.available) {
        (_, _, true) => theme.green,
        (
            crate::discord::setup::Status::NeedsToken
            | crate::discord::setup::Status::NeedsDestination
            | crate::discord::setup::Status::Checking,
            _,
            _,
        )
        | (_, Phase::Initializing, _) => theme.muted_foreground,
        _ => theme.danger,
    }
}

impl PlayerApp {
    pub(in crate::app) fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = connection(&self.state);
        let (settings_icon, settings_label) = if self.settings_open {
            (AudioIcon::ArrowLeft, "Back to player")
        } else {
            (AudioIcon::Settings, "Discord settings")
        };
        let color = connection_color(&self.state, theme);
        div()
            .id("player-header")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Grajek"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .line_height(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(5.)).flex_none().rounded_full().bg(color))
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(if self.state.available {
                                        theme.muted_foreground
                                    } else {
                                        color
                                    })
                                    .child(label("voice-status", status.into(), &self.help)),
                            ),
                    )
                    .children(self.state.setup.destination.as_ref().map(|destination| {
                        div()
                            .flex()
                            .min_w_0()
                            .text_color(theme.muted_foreground)
                            .child(label(
                                "voice-destination",
                                format!(
                                    "{} / {}",
                                    destination.guild_name, destination.channel_name
                                ),
                                &self.help,
                            ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .track_focus(&self.voice_focus)
                    .tab_stop(false)
                    .when(self.state.can_leave_voice(), |row| {
                        row.child(self.leave_button(cx))
                    })
                    .child(
                        self.focus
                            .button("settings")
                            .icon(settings_icon)
                            .accessibility_label(settings_label)
                            .help(settings_label, &self.help)
                            .ghost()
                            .flex_none()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.retire_search(cx);
                                this.help.dismiss(cx);
                                this.setup_starting = false;
                                this.settings_open = !this.settings_open;
                                this.close_popups();
                                if this.settings_open {
                                    this.focus_settings(window, cx);
                                } else {
                                    window.focus(&this.input.read(cx).focus_handle(cx), cx);
                                }
                                cx.notify();
                            })),
                    ),
            )
    }

    /// A long title can never widen the window or push the controls away.
    pub(in crate::app) fn now_playing(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let track_id = self.state.track_id;
        let resting = if self.state.error.is_some() {
            theme.danger
        } else if self.state.phase == Phase::Playing {
            theme.primary
        } else if self.state.phase == Phase::Paused {
            theme.yellow
        } else {
            theme.muted_foreground
        };
        let phase_color = self.frame.settle(track_id, theme.muted_foreground, resting);
        let track = self.state.visible_track();
        let (title, title_color) = match track {
            Some(info) => (
                info.title
                    .clone()
                    .unwrap_or_else(|| "Untitled track".into()),
                self.frame
                    .settle(track_id, theme.muted_foreground, theme.foreground),
            ),
            None => (
                match self.state.phase {
                    Phase::Ready | Phase::Error if !self.state.available => "Connect to Discord",
                    Phase::Ready => "Ready for a link",
                    Phase::Initializing => "Starting Grajek…",
                    Phase::Connecting => "Joining voice…",
                    Phase::Resolving => "Preparing your track…",
                    Phase::Stopping => "Stopping playback…",
                    Phase::Disconnecting => "Leaving voice…",
                    Phase::Error => "Playback stopped",
                    Phase::Playing | Phase::Paused => "Untitled track",
                }
                .into(),
                theme.muted_foreground,
            ),
        };
        let progress = track.map(|info| format_progress(self.state.position, info.duration));
        let symbol = state_symbol(self.state.phase);
        div().flex().flex_none().items_center().child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(heading_size(window))
                        .line_height(relative(1.25))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(title_color)
                        .child(label("title", title, &self.help))
                        .children(self.now_playing_favorite(cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .line_height(px(16.))
                        .child(Icon::new(symbol).size(px(12.)).text_color(phase_color))
                        .child(
                            div()
                                .id("seek-status")
                                .flex_none()
                                .when(
                                    track.is_some_and(|track| {
                                        track.duration.is_some_and(|duration| !duration.is_zero())
                                    }),
                                    |status| status.w(px(104.)).truncate(),
                                )
                                .role(gpui_kit::Role::Status)
                                .text_color(phase_color)
                                .when(self.progress_preview.is_some(), |status| {
                                    status.font_family(theme.mono_font_family.clone())
                                })
                                .child(self.progress_status())
                                .test_support(),
                        )
                        .children(progress.map(|progress| {
                            div()
                                .id("progress")
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.muted_foreground)
                                .child(progress)
                                .test_support()
                        }))
                        .children(self.progress_row(window, cx)),
                ),
        )
    }

    /// Keep failures readable without displacing the scrolling list.
    pub(in crate::app) fn error(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let theme = cx.theme();
        let error = motion::displayed_error(&self.state)?;
        let tip = error.clone();
        Some(
            div()
                .id("error")
                .role(gpui_kit::Role::Alert)
                .flex_none()
                .w_full()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .bg(theme.button_danger)
                .text_xs()
                .line_height(px(16.))
                .text_color(theme.danger)
                .line_clamp(2)
                .when_some(self.frame.error_fade(), Styled::opacity)
                .child(error)
                .help(tip, &self.help)
                .test_support(),
        )
    }

    pub(in crate::app) fn idle(&self) -> bool {
        self.state.visible_track().is_none()
            && matches!(self.state.phase, Phase::Ready | Phase::Error)
            && !matches!(self.pending, Some(Pending::Play | Pending::Next))
    }

    pub(in crate::app) fn active_player(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("active-player")
            .flex_none()
            .flex()
            .flex_col()
            .gap(space::TIGHT)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::TIGHT)
                    .child(self.now_playing(window, cx)),
            )
            .child(self.controls(cx))
            .children(self.error(cx))
            .test_support()
    }

    pub(in crate::app) fn idle_player(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let error = self.error(cx);
        let panel = div()
            .id("idle-player")
            .track_focus(&self.controls_focus)
            .tab_stop(false)
            .flex()
            .flex_none()
            .flex_col();
        if self.state.available {
            panel
                .gap(space::GROUP)
                .when(!self.state.queue.is_empty(), |panel| {
                    panel.child(div().flex().child(self.play_queue_button(cx)))
                })
                .children(error)
                .test_support()
                .into_any_element()
        } else {
            let (title, guidance, action) = if self.state.setup.bot.is_none() {
                (
                    "Connect your Discord bot",
                    "Set up a bot to play YouTube in your Discord voice channel.",
                    "Connect a bot",
                )
            } else {
                (
                    "Choose where to play",
                    "Select a server and voice channel in Discord settings.",
                    "Choose destination",
                )
            };
            panel
                .gap(space::DETAIL)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::TIGHT)
                        .child(
                            div()
                                .text_size(heading_size(window))
                                .line_height(relative(1.25))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label("title", title.into(), &self.help)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .line_height(px(20.))
                                .text_color(cx.theme().muted_foreground)
                                .child(guidance),
                        ),
                )
                .child(
                    div().flex().child(
                        self.focus
                            .button("setup-next-step")
                            .primary()
                            .label(action)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.retire_search(cx);
                                this.setup_starting = false;
                                this.settings_open = true;
                                this.help.dismiss(cx);
                                this.focus_settings(window, cx);
                                cx.notify();
                            })),
                    ),
                )
                .children(error)
                .test_support()
                .into_any_element()
        }
    }
}
