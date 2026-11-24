use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, button::ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use super::fmt::{label, title_or_url_id};
use super::sections::lists::{Tab, TabPrev};
use super::ui::help::HelpExt;
use super::{PlayerApp, Section};
use crate::favorites::FavoriteEntry;
use crate::player::{Command, State};
use crate::search::SearchAction;

impl PlayerApp {
    pub(super) fn is_favorite_url(&self, url: &str) -> bool {
        crate::youtube::video_url(url).is_ok_and(|canonical| self.state.is_favorite(&canonical))
    }

    pub(super) fn favorite_action_title(&self, url: &str) -> String {
        if self.is_favorite_url(url) {
            "Remove from Favorites".into()
        } else {
            "Save to Favorites".into()
        }
    }

    pub(super) fn send_set_favorite(&mut self, url: String, saved: bool, cx: &mut Context<Self>) {
        if !self.state.controller_ready || self.favorite_pending.is_some() {
            return;
        }
        let id = self.favorite_seq + 1;
        if !self.send(Command::SetFavorite { id, url, saved }) {
            self.local_hint = Some("Controller busy. Try Save again.".into());
            cx.notify();
            return;
        }
        self.favorite_seq = id;
        self.favorite_pending = Some(id);
        self.local_hint = None;
        cx.notify();
    }

    pub(super) fn send_search_favorite(
        &mut self,
        search_id: u64,
        video_id: String,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.state.controller_ready || self.favorite_pending.is_some() {
            return;
        }
        let id = self.favorite_seq + 1;
        if !self.send(Command::SearchAction {
            search_id,
            video_id,
            action: SearchAction::Favorite { id, saved },
        }) {
            self.local_hint = Some("Controller busy. Try Save again.".into());
            cx.notify();
            return;
        }
        self.favorite_seq = id;
        self.favorite_pending = Some(id);
        self.local_hint = None;
        cx.notify();
    }

    /// Resolves the membership acknowledgement and rescues focus when the current-track
    /// control disappears. Called before the snapshot is installed.
    pub(super) fn observe_favorite(
        &mut self,
        next: &State,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.favorite_seq = self
            .favorite_seq
            .max(next.favorite_result.as_ref().map_or(0, |r| r.request_id));
        if let Some(pending_id) = self.favorite_pending
            && let Some(result) = next
                .favorite_result
                .as_ref()
                .filter(|result| result.request_id == pending_id)
        {
            self.favorite_pending = None;
            match result.result {
                Err(refusal) => self.local_hint = Some(refusal.to_string()),
                Ok(()) if result.saved => {
                    self.motion
                        .saved(result.url.clone(), cx.background_executor().now());
                }
                Ok(()) => {}
            }
        }
        if self.state.current_url.is_some()
            && next.current_url.is_none()
            && self.now_playing_favorite_focus.contains_focused(window, cx)
        {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    /// The Save/Remove control beside the current track, plus the confirmed Saved label.
    pub(super) fn now_playing_favorite(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let url = self.state.current_url.clone()?;
        let saved = self.state.is_favorite(&url);
        let name = self
            .state
            .visible_track()
            .and_then(|track| track.title.clone())
            .unwrap_or_else(|| url.clone());
        let label = if saved {
            format!("Remove from Favorites: {name}")
        } else {
            format!("Save to Favorites: {name}")
        };
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .track_focus(&self.now_playing_favorite_focus)
                .tab_stop(false)
                .when(saved, |row| {
                    row.child(Self::saved_label(
                        "now-playing-saved",
                        self.saved_label_color(&url, cx),
                    ))
                })
                .child(
                    self.focus
                        .button("now-playing-favorite")
                        .label(if saved { "Remove" } else { "Save" })
                        .ghost()
                        .small()
                        .flex_none()
                        .accessibility_label(label.clone())
                        .help(label, &self.help)
                        .disabled(!self.state.controller_ready)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send_set_favorite(url.clone(), !saved, cx);
                        })),
                ),
        )
    }

    pub(super) fn saved_label(id: impl Into<ElementId>, color: gpui_kit::Hsla) -> impl IntoElement {
        div()
            .id(id)
            .flex_none()
            .text_xs()
            .line_height(px(16.))
            .text_color(color)
            .role(gpui_kit::Role::Status)
            .aria_label("Saved to Favorites")
            .child("Saved")
            .test_support()
    }

    pub(super) fn saved_label_color(&self, url: &str, cx: &Context<Self>) -> gpui_kit::Hsla {
        let canonical = crate::youtube::video_url(url).unwrap_or_else(|_| url.to_owned());
        let theme = cx.theme();
        self.frame
            .saved(&canonical, theme.muted_foreground, theme.primary)
    }

    fn favorite_title(entry: &FavoriteEntry) -> String {
        title_or_url_id(entry.title.as_deref(), &entry.url)
    }

    pub(super) fn favorite_row(
        &self,
        position: usize,
        entry: &FavoriteEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let url = entry.url.clone();
        let title = Self::favorite_title(entry);
        let play_url = url.clone();
        let play_id: ElementId = format!("favorite-play:{url}").into();
        self.track_row(
            format!("favorite-row:{url}").into(),
            &self.favorites_scroll.rows[position].1,
            window,
            cx,
        )
        .on_key_down(cx.listener(move |this, event, window, cx| {
            if this.row_menu.is_none() {
                this.list_key(Section::Favorites, position, event, window, cx);
            }
        }))
        .child(
            self.focus
                .button(play_id)
                .key_context("PlayerList")
                .on_action(cx.listener(move |this, _: &Tab, window, cx| {
                    this.list_tab(Section::Favorites, position, 0, false, window, cx);
                }))
                .on_action(cx.listener(move |this, _: &TabPrev, window, cx| {
                    this.list_tab(Section::Favorites, position, 0, true, window, cx);
                }))
                .icon(IconName::Play)
                .ghost()
                .small()
                .accessibility_label(format!("Play now: {title}. Replaces current playback"))
                .help(
                    "Play now · replaces current playback; keeps the queue",
                    &self.help,
                )
                .disabled(!self.play_enabled())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.play_url(play_url.clone(), window, cx);
                })),
        )
        .child(label(format!("favorite-title:{url}"), title, &self.help))
        .children(
            entry
                .duration_secs
                .map(|seconds| Self::duration_cell(std::time::Duration::from_secs(seconds), cx)),
        )
        .child(self.row_menu(Section::Favorites, position, position as u64, cx))
        .test_support()
    }

    pub(super) fn favorite_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let count = self.state.favorites.len();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(if count == 0 {
                super::ui::empty(
                    "favorites-empty",
                    "Keep discoveries here. Save a track from playback, Up next, History or Find Music.",
                    cx,
                )
                .into_any_element()
            } else {
                self.scroll_list(Section::Favorites, cx).into_any_element()
            })
    }
}
