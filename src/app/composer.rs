//! The music-entry composer: one input frame with Play now attached, quiet queue/import
//! actions, and a single fixed-height status line for recognition and confirmed feedback.
use gpui_kit::assets::IconName as AudioIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, button::ButtonVariants, input::Input,
};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, TestSupportExt, Window, div, prelude::FluentBuilder, px,
};

use super::help::HelpExt;
use super::{PlayerApp, label};
use crate::config::{playlist_url, video_url};
use crate::player::{Command, ImportStatus, Phase};
use crate::style::space;

/// What the pasted text is, judged locally. Recognition never starts playback or import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkKind {
    Empty,
    Video,
    /// A watch link that also names a playlist: Enter and Add still use the single video.
    VideoInPlaylist,
    Playlist,
    Other,
}

impl LinkKind {
    pub(super) fn of(text: &str) -> Self {
        let text = text.trim();
        if text.is_empty() {
            Self::Empty
        } else if text.contains("list=") && playlist_url(text).is_ok() {
            if video_url(text).is_ok() {
                Self::VideoInPlaylist
            } else {
                Self::Playlist
            }
        } else if video_url(text).is_ok() {
            Self::Video
        } else {
            Self::Other
        }
    }

    fn is_playlist(self) -> bool {
        matches!(self, Self::Playlist | Self::VideoInPlaylist)
    }

    fn message(self) -> Option<&'static str> {
        match self {
            Self::Empty | Self::Other => None,
            Self::Video => Some("Video link recognized."),
            Self::VideoInPlaylist => {
                Some("Video in a playlist · Play now and Add to queue use this video only.")
            }
            Self::Playlist => Some("Playlist link · Import playlist adds up to 100 tracks."),
        }
    }
}

/// A submitted Add to queue that the controller has not answered yet.
pub(super) struct PendingEnqueue {
    text: String,
    id: u64,
}

pub(super) enum Resolution {
    /// The controller inserted this entry.
    Added(u64),
    Failed,
}

impl PlayerApp {
    /// Add to queue for the entered link. Feedback follows the controller's snapshots only.
    pub(super) fn enqueue(&mut self, cx: &mut Context<Self>) {
        let text = self.input_text(cx);
        if text.trim().is_empty() {
            self.local_hint = Some("Paste a link first.");
            self.added = None;
            cx.notify();
            return;
        }
        if !self.state.can_enqueue()
            || self
                .enqueue_pending
                .as_ref()
                .is_some_and(|pending| pending.text == text)
        {
            return;
        }
        let id = self.enqueue_seq + 1;
        if self.send(Command::EnqueueRequest {
            id,
            value: text.clone(),
        }) {
            self.enqueue_seq = id;
            self.added = None;
            self.local_hint = None;
            self.enqueue_pending = Some(PendingEnqueue { text, id });
            cx.notify();
        }
    }

    /// A matching controller result survives coalesced snapshots and identical refusals.
    pub(super) fn resolve_enqueue(&self, next: &crate::player::State) -> Option<Resolution> {
        let pending = self.enqueue_pending.as_ref()?;
        let result = next
            .enqueue_result
            .filter(|result| result.request_id == pending.id)?;
        Some(result.item_id.map_or(Resolution::Failed, Resolution::Added))
    }

    pub(super) fn finish_enqueue(
        &mut self,
        resolution: &Resolution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.enqueue_pending.take() else {
            return;
        };
        if let Resolution::Added(id) = resolution {
            self.added = Some(*id);
            // Only the text that was submitted is cleared; anything typed since stays.
            if self.input_text(cx) == pending.text {
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }

    /// The status line: import progress, then confirmed add, then recognition.
    fn composer_status(&self, cx: &Context<Self>) -> Option<(String, bool)> {
        if self.state.import != ImportStatus::Ready
            && self.retired_import.as_ref() != Some(&self.state.import)
        {
            let failed = matches!(self.state.import, ImportStatus::Failed(_));
            return Some((self.state.import.message(), failed));
        }
        if let Some(id) = self.added
            && let Some(position) = self.state.queue.iter().position(|item| item.id == id)
        {
            return Some((
                format!(
                    "Added to queue · #{} · {}",
                    position + 1,
                    self.state.queue[position].label()
                ),
                false,
            ));
        }
        if let Some(hint) = self.local_hint {
            return Some((hint.into(), false));
        }
        LinkKind::of(&self.input_text(cx))
            .message()
            .map(|message| (message.into(), false))
    }

    /// One frame owns the boundary; Play now lives inside it so the field and its default
    /// consequence read as a single object.
    fn link_frame(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let boundary = self.focus.editing_border(focused, cx);
        let primary = self.state.available
            && !matches!(self.state.phase, Phase::Playing | Phase::Paused)
            && (!self.idle() || self.state.queue.is_empty());
        div()
            .id("url")
            .flex()
            .items_center()
            .w_full()
            .h(px(40.))
            .pr(px(3.))
            .rounded(theme.radius)
            .bg(theme.muted)
            .border_1()
            .border_color(boundary)
            .hover(move |frame| {
                frame.border_color(if focused {
                    boundary
                } else {
                    theme.muted_foreground
                })
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .h_full()
                    .child(
                        Input::new(&self.input)
                            .id("url-editor")
                            .aria_label("Paste YouTube link")
                            .bordered(false)
                            .w_full()
                            .bg(theme.muted),
                    ),
            )
            .child(
                self.button("play")
                    .when(primary, ButtonVariants::primary)
                    .icon(AudioIcon::Play)
                    .label("Play now")
                    .flex_none()
                    .help(
                        "Play this video now · Enter. Replaces what is playing; the queue is kept.",
                        &self.help,
                    )
                    .disabled(!self.play_enabled())
                    .on_click(cx.listener(|this, _, window, cx| this.play(window, cx))),
            )
            .test_support()
    }

    /// Quiet queue and import actions; the keyboard hint sits on the action it belongs to.
    fn link_actions(&self, cx: &Context<Self>) -> impl IntoElement {
        let kind = LinkKind::of(&self.input_text(cx));
        div()
            .flex()
            .items_center()
            .gap(space::GROUP)
            .child(
                self.button("enqueue")
                    .icon(AudioIcon::ListPlus)
                    .label("Add to queue")
                    .ghost()
                    .small()
                    .flex_none()
                    .disabled(!self.state.can_enqueue())
                    .help(
                        if self.state.queue.len() >= crate::queue::QUEUE_LIMIT {
                            "Queue full · remove a track to add another"
                        } else {
                            "Add this video to the end of the queue · Ctrl+Enter"
                        },
                        &self.help,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.enqueue(cx))),
            )
            .child(self.import_button(kind.is_playlist(), cx))
    }

    /// Fixed height: recognition, confirmation and import feedback never move the list.
    fn link_status(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = self.composer_status(cx);
        let added = self.added.is_some() && self.state.import == ImportStatus::Ready;
        div()
            .id("link-status")
            .role(gpui_kit::Role::Status)
            .flex()
            .items_center()
            .gap(space::TIGHT)
            .h(px(16.))
            .text_xs()
            .line_height(px(16.))
            .text_color(match &status {
                Some((_, true)) => theme.danger,
                _ => theme.muted_foreground,
            })
            .when_some(status.clone(), |line, (text, _)| {
                line.aria_label(text.clone())
                    .when(added, |line| {
                        line.child(
                            Icon::new(AudioIcon::Check)
                                .size(px(12.))
                                .text_color(theme.primary),
                        )
                    })
                    .child(label("link-status-text", text, &self.help))
            })
            .test_support()
    }

    pub(super) fn composer(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("link-entry")
            .test_support()
            .flex()
            .flex_none()
            .flex_col()
            .gap(space::TIGHT)
            .child(self.link_frame(window, cx))
            .child(self.link_actions(cx))
            .child(self.link_status(cx))
    }
}
