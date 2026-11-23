//! Fixtures and native interactions shared by more than one test module.
use super::*;
use crate::app::motion::Frame;
use crate::discord::setup::{Channel, Destination, Server, Status};
use crate::search::{Results, SearchPhase, SearchState};
use serenity::all::{ChannelId, GuildId, UserId};

pub(super) fn focused(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> bool {
    let id = id.into();
    let mut result = false;
    act(cx, window, |window, _| {
        result = window
            .try_find(id)
            .is_some_and(|button| button.focused() == Some(true));
    });
    result
}

pub(super) fn wheel_at(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    delta: f32,
) {
    act(cx, window, |window, cx| {
        window.simulate_mouse_move(position, cx);
        window.dispatch_event(
            gpui_kit::PlatformInput::ScrollWheel(gpui_kit::ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
    });
}

pub(super) fn tab_until(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) {
    let id = id.into();
    for _ in 0..40 {
        if focused(cx, window, id.clone()) {
            return;
        }
        act(cx, window, |window, cx| {
            window.press("tab", cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        });
    }
    assert!(
        focused(cx, window, id.clone()),
        "keyboard focus never reached {id:?}"
    );
}

pub(super) fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
}

pub(super) fn frame(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<PlayerApp>,
) -> Frame {
    drawn(cx, window);
    cx.update(|cx| view.read(cx).frame.clone())
}

pub(super) fn snapshot(phase: Phase, search: SearchPhase) -> State {
    State {
        search: SearchState {
            id: 1,
            query: "Bach cello".into(),
            phase: search,
            status: None,
        },
        ..state(phase)
    }
}

pub(super) fn results() -> SearchPhase {
    SearchPhase::Results(Results {
        items: (0..5)
            .map(|index| crate::search::ResultItem {
                url: format!("https://www.youtube.com/watch?v=aaaaaaaaaa{index}"),
                title: Some(format!(
                    "巴赫 · Żółć 🎵 {index} {}",
                    "Factual title ".repeat(28)
                )),
                channel: Some("Channel · 大提琴".into()),
                duration: Some(Duration::from_secs(225)),
                live: false,
            })
            .collect(),
        rejected: 0,
    })
}

pub(super) fn configured() -> State {
    let mut snapshot = state(Phase::Ready);
    snapshot.setup.status = Status::Ready;
    snapshot.setup.bot = Some(UserId::new(1));
    snapshot.setup.browsing = Some(GuildId::new(2));
    snapshot.setup.servers = vec![
        Server {
            id: GuildId::new(2),
            name: "Friends’ listening room".into(),
            available: true,
        },
        Server {
            id: GuildId::new(4),
            name: "Pracownia · 音楽と友達".into(),
            available: true,
        },
        Server {
            id: GuildId::new(5),
            name: "Server temporarily offline".into(),
            available: false,
        },
    ];
    snapshot.setup.channels = vec![
        Channel {
            id: ChannelId::new(3),
            name: "Music room".into(),
            reason: None,
        },
        Channel {
            id: ChannelId::new(6),
            name: "夜の音楽 · Żółć · Привет · A very long channel name for listening together"
                .into(),
            reason: None,
        },
        Channel {
            id: ChannelId::new(7),
            name: "Private listening room".into(),
            reason: Some("Needs View Channel, Connect and Speak".into()),
        },
        Channel {
            id: ChannelId::new(8),
            name: "Live stage".into(),
            reason: Some("Stage channels are not supported".into()),
        },
    ];
    snapshot.setup.destination = Some(Destination {
        bot: UserId::new(1),
        guild: GuildId::new(2),
        channel: ChannelId::new(3),
        guild_name: "Friends’ listening room".into(),
        channel_name: "Music room".into(),
    });
    snapshot
}
