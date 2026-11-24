//! The music-entry composer: one input frame with Play now attached, quiet queue/import
//! actions, and a single fixed-height status line for recognition and confirmed feedback.
mod import;
mod link;

use gpui_kit::assets::IconName as AudioIcon;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, button::ButtonVariants, input::Input,
};
use gpui_kit::{
    Context, Focusable, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, TestSupportExt, Window, div, prelude::FluentBuilder, px,
};

use super::PlayerApp;
use super::fmt::label;
use super::ui::help::HelpExt;
use crate::style::space;
pub(super) use link::LinkKind;

use crate::{
    player::{Command, Phase, Placement, RepeatMode},
    playlist::ImportStatus,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Landing {
    End,
    Next,
    Moved,
}

impl From<Placement> for Landing {
    fn from(placement: Placement) -> Self {
        match placement {
            Placement::End => Self::End,
            Placement::Next => Self::Next,
        }
    }
}

pub(super) const REPEAT_TRACK_NOTE: &str = "Repeat Track is on; press Next to play it.";

/// An insert or move the controller has not answered yet. `text` is the composer text it was
/// submitted from; `None` marks a list action, which typing must not cancel.
pub(super) struct PendingEnqueue {
    pub(super) text: Option<String>,
    id: u64,
    landing: Landing,
}

pub(super) enum Resolution {
    Placed(u64, Landing),
    Failed,
}

impl PlayerApp {
    /// Feedback follows the controller's snapshots only.
    pub(super) fn enqueue(&mut self, placement: Placement, cx: &mut Context<Self>) {
        let text = self.input_text(cx);
        if LinkKind::of(&text) == LinkKind::Query {
            self.local_hint = Some("Search, then select a result to add or queue next.".into());
            cx.notify();
            return;
        }
        if text.trim().is_empty() {
            self.local_hint = Some("Paste a link first.".into());
            self.added = None;
            cx.notify();
            return;
        }
        if !self.can_place(placement)
            || self
                .enqueue_pending
                .as_ref()
                .is_some_and(|pending| pending.text.as_ref() == Some(&text))
        {
            return;
        }
        if self.begin_request(Some(text.clone()), placement.into(), |id| {
            Command::EnqueueRequest {
                id,
                value: text,
                placement,
            }
        }) {
            self.local_hint = None;
            cx.notify();
        }
    }

    /// Sends one insert or move request and holds the window's only pending slot for it.
    fn begin_request(
        &mut self,
        text: Option<String>,
        landing: Landing,
        command: impl FnOnce(u64) -> Command,
    ) -> bool {
        let id = self.enqueue_seq + 1;
        if !self.send(command(id)) {
            return false;
        }
        self.enqueue_seq = id;
        self.added = None;
        self.enqueue_pending = Some(PendingEnqueue { text, id, landing });
        true
    }

    pub(super) fn can_place(&self, placement: Placement) -> bool {
        match placement {
            Placement::End => self.state.can_enqueue(),
            Placement::Next => self.state.can_queue_next(),
        }
    }

    pub(super) fn enqueue_url(&mut self, url: String, placement: Placement) {
        if self.enqueue_pending.is_none() && self.can_place(placement) {
            self.begin_request(None, placement.into(), |id| Command::EnqueueRequest {
                id,
                value: url,
                placement,
            });
        }
    }

    pub(super) fn move_to_next(&mut self, entry: u64) {
        if self.enqueue_pending.is_none() && self.state.controller_ready {
            self.begin_request(None, Landing::Moved, |request_id| Command::MoveToNext {
                request_id,
                id: entry,
            });
        }
    }

    /// A matching controller result survives coalesced snapshots and identical refusals.
    pub(super) fn resolve_enqueue(&self, next: &crate::player::State) -> Option<Resolution> {
        let pending = self.enqueue_pending.as_ref()?;
        let result = next
            .enqueue_result
            .filter(|result| result.request_id == pending.id)?;
        Some(result.item_id.map_or(Resolution::Failed, |id| {
            Resolution::Placed(id, pending.landing)
        }))
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
        if let Resolution::Placed(id, landing) = resolution {
            self.added = Some((*id, *landing));
            self.motion.added(*id, cx.background_executor().now());
            // Only the text that was submitted is cleared; anything typed since stays.
            if let Some(text) = pending.text
                && self.input_text(cx) == text
            {
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
        if let Some((id, landing)) = self.added
            && let Some(position) = self.state.queue.iter().position(|item| item.id == id)
        {
            let title = self.state.queue[position].label();
            let note = if self.state.preferences.repeat == RepeatMode::Track {
                format!("{REPEAT_TRACK_NOTE} · ")
            } else {
                String::new()
            };
            match landing {
                Landing::End => {
                    return Some((
                        format!("Added to queue · #{} · {title}", position + 1),
                        false,
                    ));
                }
                Landing::Next if position == 0 => {
                    return Some((format!("Queued next · {note}{title}"), false));
                }
                Landing::Moved if position == 0 => {
                    return Some((format!("Moved to next · {note}{title}"), false));
                }
                Landing::Next | Landing::Moved => {}
            }
        }
        if let Some(hint) = &self.local_hint {
            return Some((hint.clone(), false));
        }
        LinkKind::of(&self.input_text(cx))
            .message()
            .map(|message| (message, false))
    }

    /// One frame owns the boundary; Play now lives inside it so the field and its default
    /// consequence read as a single object.
    fn link_frame(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let query = LinkKind::of(&self.input_text(cx)) == LinkKind::Query;
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
                            .aria_label("YouTube link or search query")
                            .bordered(false)
                            .w_full()
                            .bg(theme.muted),
                    ),
            )
            .child(
                self.focus.button("play")
                    .when(primary, ButtonVariants::primary)
                    .icon(AudioIcon::Play)
                    .label(if query { "Search YouTube" } else { "Play now" })
                    .flex_none()
                    .help(
                        if query { "Search YouTube · Enter. Select a result, then Play now or Add to queue." } else { "Play this video now · Enter. Replaces what is playing; the queue is kept." },
                        &self.help,
                    )
                    .disabled(if query { !self.state.controller_ready } else { !self.play_enabled() })
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
                self.focus
                    .button("enqueue")
                    .icon(AudioIcon::ListPlus)
                    .label("Add to queue")
                    .ghost()
                    .small()
                    .flex_none()
                    .disabled(!self.state.can_enqueue())
                    .help(
                        if crate::queue::is_full(self.state.queue.len()) {
                            "Queue full · remove a track to add another"
                        } else {
                            "Add this video to the end of the queue · Ctrl+Enter"
                        },
                        &self.help,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.enqueue(Placement::End, cx))),
            )
            .child(
                self.focus
                    .button("enqueue-next")
                    .icon(AudioIcon::ListStart)
                    .accessibility_label("Queue next")
                    .ghost()
                    .small()
                    .flex_none()
                    .disabled(!self.state.can_queue_next())
                    .help(self.queue_next_help(), &self.help)
                    .on_click(cx.listener(|this, _, _, cx| this.enqueue(Placement::Next, cx))),
            )
            .child(self.import_button(kind.is_playlist(), cx))
    }

    pub(super) fn queue_next_help(&self) -> String {
        if crate::queue::is_full(self.state.queue.len()) {
            return "Queue full · remove a track to queue another".into();
        }
        let mut help =
            String::from("Queue next · plays after the current track. Replaces nothing.");
        if self.state.preferences.repeat == RepeatMode::Track {
            help.push(' ');
            help.push_str(REPEAT_TRACK_NOTE);
        }
        help
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
