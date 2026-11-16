use super::{Pending, PlayerApp, clock, help::HelpExt, label};
use crate::player::{Command, SearchAction, SearchPhase, State};
use crate::search::ResultItem;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, button::ButtonVariants, scroll::ScrollableElement,
};
use gpui_kit::{
    AnyElement, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Subscription, TestSupportExt,
    Window, div, prelude::FluentBuilder, px,
};

#[derive(Default)]
pub(super) struct SearchUi {
    seq: u64,
    current: Option<Session>,
    scroll: ScrollHandle,
}

struct Session {
    id: u64,
    query: String,
    selected: Option<String>,
    action: Option<SearchAction>,
    rows: Vec<FocusHandle>,
    _focus: Vec<Subscription>,
}

impl PlayerApp {
    pub(super) fn searching(&self) -> bool {
        self.search_ui.current.is_some()
    }

    pub(super) fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(query) = crate::search::query(&self.input_text(cx)) else {
            self.local_hint =
                Some("Enter a search query of at most 200 characters, without control characters.");
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
            self.local_hint = Some("Controller busy. Try Search YouTube again.");
            cx.notify();
            return;
        }
        self.search_ui.seq = id;
        self.search_ui.scroll = ScrollHandle::default();
        let rows: Vec<_> = (0..5).map(|_| cx.focus_handle().tab_stop(true)).collect();
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
        self.row_menu = None;
        self.queue_options_open = false;
        self.confirm_clear = false;
        self.local_hint = None;
        self.added = None;
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn retire_search(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.search_ui.current.take() else {
            return false;
        };
        if !self.send(Command::CancelSearch { id: session.id }) {
            self.local_hint = Some("Controller busy. Search is closed; try again shortly.");
        }
        cx.notify();
        true
    }

    pub(super) fn search_escape(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "escape"
            && !self.help.dismiss_if_active(cx)
            && !self.queue_options_open
            && self.row_menu.is_none()
        {
            self.close_search(window, cx);
        }
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.retire_search(cx) {
            window.focus(&self.input.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn observe_search(
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
            if let Some(SearchAction::Enqueue { id }) = session.action
                && let Some(result) = next.enqueue_result.filter(|result| result.request_id == id)
            {
                self.added = result.item_id;
            }
            self.search_ui.current = None;
            if !self.settings_open && !self.components.open {
                window.focus(&self.input.read(cx).focus_handle(cx), cx);
            }
        } else if match session.action {
            Some(SearchAction::Play) => next.transport_ack >= self.transport_seq,
            Some(SearchAction::Enqueue { id }) => next
                .enqueue_result
                .is_some_and(|result| result.request_id == id),
            None => false,
        } {
            session.action = None;
        }
    }

    fn search_phase(&self) -> Option<&SearchPhase> {
        let session = self.search_ui.current.as_ref()?;
        (session.id == self.state.search.id && session.query == self.state.search.query)
            .then_some(&self.state.search.phase)
    }

    fn selected_search_item(&self) -> Option<&ResultItem> {
        let selected = self.search_ui.current.as_ref()?.selected.as_ref()?;
        let SearchPhase::Results(results) = self.search_phase()? else {
            return None;
        };
        results.items.iter().find(|item| item.id() == selected)
    }

    fn select_search(&mut self, video_id: String, cx: &mut Context<Self>) {
        if let Some(session) = &mut self.search_ui.current {
            session.selected = Some(video_id);
        }
        cx.notify();
    }

    fn apply_search_action(&mut self, play: bool, cx: &mut Context<Self>) {
        let Some(item) = self.selected_search_item() else {
            return;
        };
        let video_id = item.id().to_owned();
        let Some(session) = &self.search_ui.current else {
            return;
        };
        if session.action.is_some()
            || (play && !self.play_enabled())
            || (!play && !self.state.can_enqueue())
        {
            return;
        }
        let search_id = session.id;
        let action = if play {
            SearchAction::Play
        } else {
            SearchAction::Enqueue {
                id: self.enqueue_seq + 1,
            }
        };
        let command = Command::SearchAction {
            search_id,
            video_id,
            action,
        };
        let sent = if play {
            self.send_transport(command, Pending::Play)
        } else {
            self.send(command)
        };
        if sent {
            if let SearchAction::Enqueue { id } = action {
                self.enqueue_seq = id;
            }
            if let Some(session) = &mut self.search_ui.current {
                session.action = Some(action);
            }
            self.local_hint = None;
        } else {
            self.local_hint = Some("Controller busy. Try the selected action again.");
        }
        cx.notify();
    }

    fn search_row_key(
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

    fn search_row(&self, index: usize, item: &ResultItem, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(session) = &self.search_ui.current else {
            return div().into_any_element();
        };
        let selected = session.selected.as_deref() == Some(item.id());
        let title = item.title.as_deref().unwrap_or("Title unavailable");
        let channel = item.channel.as_deref().unwrap_or("Channel unavailable");
        let duration = if item.live {
            "Live".into()
        } else {
            item.duration
                .map_or_else(|| "Duration unavailable".into(), clock)
        };
        let full = format!("{title} · {channel} · {duration}");
        let video_id = item.id().to_owned();
        div()
            .id(format!("search-row-{index}"))
            .role(gpui_kit::Role::ListBoxOption)
            .aria_label(full.clone())
            .aria_selected(selected)
            .test_support()
            .track_focus(&session.rows[index])
            .tab_stop(true)
            .cursor_pointer()
            .flex_none()
            .h(px(44.))
            .w_full()
            .min_w_0()
            .px_2()
            .flex()
            .flex_col()
            .justify_center()
            .rounded(theme.radius)
            .when(selected, |row| row.bg(theme.accent))
            .hover(|row| row.bg(theme.muted))
            .focus_visible(|row| row.border_1().border_color(theme.ring))
            .help(full, &self.help)
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .text_sm()
                    .line_height(px(20.))
                    .child(label(
                        format!("search-title-{index}"),
                        title.into(),
                        &self.help,
                    )),
            )
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .gap_2()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(channel.to_owned()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme.mono_font_family.clone())
                            .child(duration),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(session) = &this.search_ui.current {
                    window.focus(&session.rows[index], cx);
                }
                this.select_search(video_id.clone(), cx);
            }))
            .on_key_down(cx.listener(move |this, event, window, cx| {
                this.search_row_key(index, event, window, cx);
            }))
            .into_any_element()
    }

    pub(super) fn search_collection(&self, cx: &Context<Self>) -> AnyElement {
        let phase = self.search_phase();
        let message = match phase {
            Some(SearchPhase::Results(results)) => {
                format!("{} videos · select a result", results.items.len())
            }
            Some(SearchPhase::Empty { rejected: 0 }) => {
                "No videos found. Edit your query and search again.".into()
            }
            Some(SearchPhase::Empty { .. }) => {
                "No usable videos returned. Edit your query and search again.".into()
            }
            Some(SearchPhase::Failed(error)) => error.to_string(),
            _ => "Searching YouTube…".into(),
        };
        let selected = self.selected_search_item();
        let pending = self
            .search_ui
            .current
            .as_ref()
            .is_some_and(|session| session.action.is_some());
        let title = selected
            .and_then(|item| item.title.as_deref())
            .unwrap_or("Title unavailable");
        let failed = matches!(phase, Some(SearchPhase::Failed(_)));
        let status = self.local_hint.map(str::to_owned).or_else(|| {
            self.state
                .search
                .status
                .as_ref()
                .filter(|_| phase.is_some())
                .cloned()
        });
        let mut body = div()
            .id("search-list")
            .role(if matches!(phase, Some(SearchPhase::Results(_))) {
                gpui_kit::Role::ListBox
            } else {
                gpui_kit::Role::Group
            })
            .aria_label("YouTube search results")
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_y_scroll()
            .track_scroll(&self.search_ui.scroll)
            .pr_3();
        if let Some(SearchPhase::Results(results)) = phase {
            body = body.children(
                results
                    .items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| self.search_row(index, item, cx)),
            );
        } else {
            body = body.child(
                div()
                    .id("search-message")
                    .role(if failed {
                        gpui_kit::Role::Alert
                    } else {
                        gpui_kit::Role::Status
                    })
                    .aria_label(message.clone())
                    .text_sm()
                    .text_color(if failed {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(message.clone())
                    .test_support(),
            );
        }
        div().id("search-region").flex_1().min_h_0().flex().flex_col().gap_1()
            .child(div().flex_none().h(px(30.)).flex().items_center().justify_between().min_w_0()
                .child(div().id("search-status").role(gpui_kit::Role::Status).min_w_0().flex_1().text_sm()
                    .aria_label(if let Some(session) = &self.search_ui.current { format!("YouTube search: {} · {message}", session.query) } else { message.clone() })
                    .child(label("search-heading", if matches!(phase, Some(SearchPhase::Results(_))) { message } else { "YouTube search".into() }, &self.help)).test_support())
                .child(self.button("search-cancel").label(if matches!(phase, None | Some(SearchPhase::Loading)) { "Cancel" } else { "Back" })
                    .ghost().small().on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx)))))
            .child(body.vertical_scrollbar(&self.search_ui.scroll).test_support())
            .when(matches!(phase, Some(SearchPhase::Failed(crate::search::Error::MissingComponents))), |region| region.child(
                self.button("search-components").label("Open playback components").on_click(cx.listener(|this, _, _, cx| this.open_components(cx)))))
            .when_some(status, |region, status| region.child(
                div().id("search-action-status").role(gpui_kit::Role::Alert).aria_label(status.clone()).text_xs().child(status).test_support()))
            .when(matches!(phase, Some(SearchPhase::Results(_))), |region| region.child(
                div().id("search-actions").flex_none().h(px(32.)).flex().gap_2().test_support()
                    .child(self.button("search-play").label("Play now").accessibility_label(format!("Play now: {title}. Replaces current playback; keeps the queue."))
                        .help(format!("Play now: {title}. Replaces current playback; keeps the queue."), &self.help)
                        .disabled(selected.is_none() || pending || !self.play_enabled())
                        .on_click(cx.listener(|this, _, _, cx| this.apply_search_action(true, cx))))
                    .child(self.button("search-enqueue").label("Add to queue").accessibility_label(format!("Add to queue: {title}"))
                        .help(format!("Add to queue: {title}. Appends without interrupting playback."), &self.help)
                        .disabled(selected.is_none() || pending || !self.state.can_enqueue())
                        .on_click(cx.listener(|this, _, _, cx| this.apply_search_action(false, cx))))))
            .test_support().into_any_element()
    }
}
