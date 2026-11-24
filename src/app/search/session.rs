//! Search session state: submitting, retiring and reconciling a controller-confirmed search.

use super::super::PlayerApp;
use super::super::transport::Pending;
use crate::player::{Command, Placement, State};
use crate::search::{ResultItem, SearchAction, SearchPhase};
use gpui_kit::{Context, FocusHandle, Focusable, KeyDownEvent, ScrollHandle, Subscription, Window};

#[derive(Default)]
pub(in crate::app) struct SearchUi {
    seq: u64,
    pub(super) current: Option<Session>,
    pub(super) scroll: ScrollHandle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Play,
    Enqueue(Placement),
}

pub(super) struct Session {
    pub(super) id: u64,
    pub(super) query: String,
    pub(super) selected: Option<String>,
    pub(super) action: Option<SearchAction>,
    pub(super) rows: Vec<FocusHandle>,
    pub(super) _focus: Vec<Subscription>,
}

impl PlayerApp {
    pub(in crate::app) fn searching(&self) -> bool {
        self.search_ui.current.is_some()
    }

    pub(in crate::app) fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(query) = crate::search::query(&self.input_text(cx)) else {
            self.local_hint = Some(format!(
                "Enter a search query of at most {} characters, without control characters.",
                crate::search::QUERY_LIMIT
            ));
            cx.notify();
            return;
        };
        if !self.state.controller_ready {
            return;
        }
        if self.search_ui.current.as_ref().is_some_and(|session| {
            session.query == query
                && self
                    .search_phase()
                    .is_none_or(|phase| *phase == SearchPhase::Loading)
        }) {
            return;
        }
        let id = self.search_ui.seq.max(self.state.search.id) + 1;
        if !self.send(Command::Search {
            id,
            query: query.clone(),
        }) {
            self.local_hint = Some("Controller busy. Try Search YouTube again.".into());
            cx.notify();
            return;
        }
        self.search_ui.seq = id;
        self.search_ui.scroll = ScrollHandle::default();
        let rows: Vec<_> = (0..crate::search::ENTRY_LIMIT)
            .map(|_| cx.focus_handle().tab_stop(true))
            .collect();
        let focus = rows
            .iter()
            .enumerate()
            .map(|(index, handle)| {
                cx.on_focus(handle, window, move |this, _, cx| {
                    this.search_ui.scroll.scroll_to_item(index);
                    cx.notify();
                })
            })
            .collect();
        self.search_ui.current = Some(Session {
            id,
            query,
            selected: None,
            action: None,
            rows,
            _focus: focus,
        });
        self.close_popups();
        self.confirm_clear = false;
        self.local_hint = None;
        self.added = None;
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(in crate::app) fn retire_search(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.search_ui.current.take() else {
            return false;
        };
        if !self.send(Command::CancelSearch { id: session.id }) {
            self.local_hint = Some("Controller busy. Search is closed; try again shortly.".into());
        }
        cx.notify();
        true
    }

    pub(in crate::app) fn search_escape(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if super::super::ui::escape(&event.keystroke.key)
            && !self.help.dismiss_if_active(cx)
            && !self.queue_options_open
            && self.row_menu.is_none()
        {
            self.close_search(window, cx);
        }
    }

    pub(in crate::app) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.retire_search(cx) {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(in crate::app) fn observe_search(
        &mut self,
        next: &State,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_ui.seq = self.search_ui.seq.max(next.search.id);
        let Some(session) = &mut self.search_ui.current else {
            return;
        };
        if session.id != next.search.id || session.query != next.search.query {
            return;
        }
        if next.search.phase == SearchPhase::Idle {
            if let Some(SearchAction::Enqueue { id, placement }) = session.action
                && let Some(result) = next.enqueue_result.filter(|result| result.request_id == id)
            {
                self.added = result.item_id.map(|id| (id, placement.into()));
                if let Some(added) = result.item_id {
                    self.motion.added(added, cx.background_executor().now());
                }
            }
            self.search_ui.current = None;
            if !self.settings_open && !self.components.open {
                window.focus(&self.input.read(cx).focus_handle(cx), cx);
            }
        } else if match session.action {
            Some(SearchAction::Play) => next.transport_ack >= self.transport_seq,
            Some(SearchAction::Enqueue { id, .. }) => next
                .enqueue_result
                .is_some_and(|result| result.request_id == id),
            Some(SearchAction::Favorite { .. }) | None => false,
        } {
            session.action = None;
        }
    }

    pub(super) fn search_phase(&self) -> Option<&SearchPhase> {
        let session = self.search_ui.current.as_ref()?;
        (session.id == self.state.search.id && session.query == self.state.search.query)
            .then_some(&self.state.search.phase)
    }

    pub(super) fn selected_search_item(&self) -> Option<&ResultItem> {
        let selected = self.search_ui.current.as_ref()?.selected.as_ref()?;
        let SearchPhase::Results(results) = self.search_phase()? else {
            return None;
        };
        results.items.iter().find(|item| item.id() == selected)
    }

    pub(super) fn select_search(&mut self, video_id: String, cx: &mut Context<Self>) {
        if let Some(session) = &mut self.search_ui.current {
            session.selected = Some(video_id);
        }
        cx.notify();
    }

    pub(super) fn apply_search_action(&mut self, choice: Choice, cx: &mut Context<Self>) {
        let Some(item) = self.selected_search_item() else {
            return;
        };
        let video_id = item.id().to_owned();
        let Some(session) = &self.search_ui.current else {
            return;
        };
        let allowed = match choice {
            Choice::Play => self.play_enabled(),
            Choice::Enqueue(placement) => self.can_place(placement),
        };
        if session.action.is_some() || !allowed {
            return;
        }
        let search_id = session.id;
        let action = match choice {
            Choice::Play => SearchAction::Play,
            Choice::Enqueue(placement) => SearchAction::Enqueue {
                id: self.enqueue_seq + 1,
                placement,
            },
        };
        let command = Command::SearchAction {
            search_id,
            video_id,
            action,
        };
        let sent = if choice == Choice::Play {
            self.send_transport(command, Pending::Play)
        } else {
            self.send(command)
        };
        if sent {
            if let SearchAction::Enqueue { id, .. } = action {
                self.enqueue_seq = id;
            }
            if let Some(session) = &mut self.search_ui.current {
                session.action = Some(action);
            }
            self.local_hint = None;
        } else {
            self.local_hint = Some("Controller busy. Try the selected action again.".into());
        }
        cx.notify();
    }

    pub(super) fn search_row_key(
        &mut self,
        index: usize,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.shift
        {
            return;
        }
        let Some(SearchPhase::Results(results)) = self.search_phase() else {
            return;
        };
        let last = results.items.len().saturating_sub(1);
        let next = match event.keystroke.key.as_str() {
            "up" => index.saturating_sub(1),
            "down" => (index + 1).min(last),
            "home" => 0,
            "end" => last,
            "enter" | "space" => {
                let id = results.items[index].id().to_owned();
                self.select_search(id, cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        if let Some(session) = &self.search_ui.current {
            self.search_ui.scroll.scroll_to_item(next);
            window.focus(&session.rows[next], cx);
            cx.notify();
        }
        cx.stop_propagation();
    }
}
