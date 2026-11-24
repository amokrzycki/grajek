//! Playlist import: the composer action and its controller request.

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, button::ButtonVariants};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, ParentElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::player::Command;
use crate::playlist::ImportStatus;

use super::super::ui::help::HelpExt;
use super::super::ui::menu::Activate;
use super::PlayerApp;

impl PlayerApp {
    fn import_playlist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.import_pending {
            return;
        }
        let running = self.state.import == ImportStatus::Importing;
        if !running && !self.state.can_import() {
            return;
        }
        let command = if running {
            Command::CancelImport
        } else {
            Command::ImportPlaylist(self.input_text(cx))
        };
        if self.send(command) {
            self.retired_import = None;
            self.import_pending = true;
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
            cx.notify();
        }
    }

    /// Import playlist, or Cancel import while one runs. A playlist link in the field gives it
    /// the accent; otherwise it stays a quiet action beside Add to queue.
    pub(super) fn import_button(
        &self,
        playlist_link: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let running = self.state.import == ImportStatus::Importing;
        let label = if running {
            "Cancel import"
        } else {
            "Import playlist"
        };
        div()
            .flex_none()
            .track_focus(&self.import_focus)
            .tab_stop(false)
            .child(
                self.focus
                    .button("import-playlist")
                    .key_context("PlayerMenuControl")
                    .on_action(cx.listener(|this, _: &Activate, window, cx| {
                        this.import_playlist(window, cx);
                    }))
                    .icon(if running {
                        IconName::Close
                    } else {
                        IconName::Import
                    })
                    .label(label)
                    .accessibility_label(label)
                    .help(
                        if running {
                            "Stop the playlist import; tracks already found stay queued.".to_owned()
                        } else {
                            format!(
                                "Add the playlist in the link to the queue, up to {} tracks. Never interrupts playback.",
                                crate::playlist::ENTRY_LIMIT
                            )
                        },
                        &self.help,
                    )
                    .ghost()
                    .small()
                    .when(playlist_link && !running, |button| {
                        button.text_color(cx.theme().primary)
                    })
                    .disabled(self.import_pending || (!running && !self.state.can_import()))
                    .on_click(cx.listener(|this, _, window, cx| this.import_playlist(window, cx))),
            )
    }
}
