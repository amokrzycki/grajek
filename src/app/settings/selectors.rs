//! Destination selectors: the server/channel choice list and its controller sync.

use gpui_kit::component::{
    ActiveTheme, IndexPath, Sizable,
    select::{SelectDelegate, SelectEvent, SelectItem, SelectState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};

use crate::discord::setup::{Destination, Request};
use crate::player::Command;

use super::PlayerApp;

#[derive(Clone)]
pub(in crate::app) struct Choice {
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

pub(in crate::app) struct Choices(Vec<Choice>);
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
        if changes.iter().any(|change| {
            matches!(change, SearchableListChange::Select { index }
                if self.item(*index).is_some_and(SelectItem::disabled))
        }) {
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

pub(in crate::app) type DestinationSelect = Entity<SelectState<Choices>>;

pub(in crate::app) fn selectors(
    window: &mut Window,
    cx: &mut App,
) -> (DestinationSelect, DestinationSelect) {
    (
        cx.new(|cx| SelectState::new(Choices(Vec::new()), None, window, cx)),
        cx.new(|cx| SelectState::new(Choices(Vec::new()), None, window, cx)),
    )
}

impl PlayerApp {
    pub(in crate::app) fn subscribe_selectors(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(in crate::app) fn sync_selectors(&self, window: &mut Window, cx: &mut Context<Self>) {
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
}
