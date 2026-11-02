use super::*;
use crate::setup::{Channel, Destination, Server, Status};
use serenity::all::{ChannelId, GuildId, UserId};

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

#[gpui_kit::test]
fn idle_has_a_next_action_instead_of_disabled_transport(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, _, _, updates), _) = open_at(cx, configured(), width, height);
        assert!(!present(cx, window, "pause-resume"));
        assert!(!present(cx, window, "stop"));
        assert!(
            !present(cx, window, "seek-status"),
            "idle must not repeat Ready"
        );
        assert!(present(cx, window, "play"));
        let mut queued = configured();
        queued.queue = with_lists(Phase::Ready, 2, 0).queue;
        updates.send_replace(queued);
        cx.run_until_parked();
        assert_eq!(button(cx, window, "next"), ("Play Queue".into(), false));
    }
}

#[gpui_kit::test]
fn configured_settings_lead_with_selectors_and_disclose_credentials(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, configured());
    click(cx, window, "settings");
    assert!(present(cx, window, "server-selector"));
    assert!(present(cx, window, "channel-selector"));
    assert!(!present(cx, window, "bot-token"));
    assert!(!present(cx, window, "remove-token"));
    click(cx, window, "change-credentials");
    assert!(present(cx, window, "bot-token"));
    assert!(
        received.try_recv().is_err(),
        "disclosure must not reconnect"
    );
}

#[gpui_kit::test]
fn server_browsing_and_channel_confirmation_keep_the_active_destination_truthful(
    cx: &mut TestAppContext,
) {
    let (window, view, mut received, updates) = open(cx, configured());
    click(cx, window, "settings");
    act(cx, window, |window, cx| {
        window.press("enter", cx);
        window.press("down", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    assert!(
        matches!(received.try_recv(), Ok(Command::Setup(crate::setup::Request::Channels(id))) if id == GuildId::new(4))
    );
    let mut browsing = configured();
    browsing.setup.browsing = Some(GuildId::new(4));
    updates.send_replace(browsing.clone());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| view
            .read(cx)
            .state
            .setup
            .destination
            .as_ref()
            .map(|d| d.channel)),
        Some(ChannelId::new(3))
    );
    click(cx, window, "channel-selector");
    assert!(present(cx, window, ("destination-choice", 7_u64)));
    act(cx, window, |window, cx| {
        window.click(("destination-choice", 7_u64), cx);
    });
    assert!(
        received.try_recv().is_err(),
        "unavailable channels cannot select or join"
    );
    act(cx, window, |window, cx| {
        assert!(
            view.read(cx)
                .channel_select
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx),
            "disabled click must keep selection focus"
        );
        assert!(
            window.try_find(("destination-choice", 6_u64)).is_some(),
            "disabled click must keep the selector open"
        );
    });
    act(cx, window, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    let selected = match received.try_recv() {
        Ok(Command::Setup(crate::setup::Request::Select(destination))) => {
            Some((destination.guild.get(), destination.channel.get()))
        }
        _ => None,
    };
    assert_eq!(selected, Some((4, 6)));
    assert_eq!(
        cx.update(|cx| view
            .read(cx)
            .state
            .setup
            .destination
            .as_ref()
            .map(|d| d.channel)),
        Some(ChannelId::new(3))
    );
    browsing.confirm_setup = true;
    updates.send_replace(browsing.clone());
    cx.run_until_parked();
    act(cx, window, |window, _| {
        assert_eq!(window.find("confirm-setup").focused(), Some(true));
    });
    click(cx, window, "cancel-setup");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::ConfirmSetup(false))
    ));
    browsing.confirm_setup = false;
    updates.send_replace(browsing);
    cx.run_until_parked();
    assert!(present(cx, window, "server-selector"));
}

#[gpui_kit::test]
fn help_wraps_inside_both_windows_and_cancels_when_a_menu_opens(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let mut snapshot = playing(
            Phase::Playing,
            Some(&"Long Unicode title · Żółć · 音楽 ".repeat(10)),
            5,
        );
        snapshot.queue = with_lists(Phase::Playing, 2, 0).queue;
        let ((window, _, _, _), _) = open_at(cx, snapshot, width, height);
        act(cx, window, |window, cx| window.hover("title", cx));
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        let tip = bounds(cx, window, "help-popup");
        assert!(tip.size.width <= px(300.) && tip.size.height > px(40.));
        assert!(
            tip.left() >= px(0.)
                && tip.right() <= px(width)
                && tip.top() >= px(0.)
                && tip.bottom() <= px(height)
        );
        click(cx, window, "queue-options");
        assert!(!present(cx, window, "help-popup"));
        act(cx, window, |window, cx| window.hover("title", cx));
        click(cx, window, "queue-options");
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(
            !present(cx, window, "help-popup"),
            "opening a menu cancels delayed help too"
        );
        for id in ["continue-on-error", "remember-queue", "repeat"] {
            act(cx, window, |window, cx| window.hover(id, cx));
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            assert!(
                !present(cx, window, "help-popup"),
                "decision help must stay inside the menu"
            );
        }
        for _ in 0..4 {
            act(cx, window, |window, cx| window.press("escape", cx));
            click(cx, window, "queue-options");
        }
        let menu = bounds(cx, window, "queue-options-menu");
        assert!(
            menu.left() >= px(0.)
                && menu.right() <= px(width)
                && menu.top() >= px(0.)
                && menu.bottom() <= px(height)
        );
        act(cx, window, |window, cx| window.press("escape", cx));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(!present(cx, window, "help-popup"));
    }
}

#[gpui_kit::test]
fn focused_control_help_is_available_from_f1_and_escape(cx: &mut TestAppContext) {
    let (window, _, _, _) = open(cx, playing(Phase::Playing, Some("Track"), 2));
    act(cx, window, |window, cx| {
        for _ in 0..20 {
            if window.find("stop").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        window.press("f1", cx);
    });
    assert!(present(cx, window, "help-popup"));
    act(cx, window, |window, cx| window.press("escape", cx));
    assert!(!present(cx, window, "help-popup"));
}

#[gpui_kit::test]
fn help_flips_and_clamps_at_every_window_corner(cx: &mut TestAppContext) {
    for (width, height) in [(520., 560.), (420., 480.)] {
        let ((window, view, _, _), _) = open_at(cx, configured(), width, height);
        for (x, y) in [
            (4., 4.),
            (width - 24., 4.),
            (4., height - 24.),
            (width - 24., height - 24.),
        ] {
            act(cx, window, |window, cx| {
                let help = view.read(cx).help.clone();
                help.show(Bounds::new(point(px(x), px(y)), size(px(20.), px(20.))), "A long description with Unicode · 音楽 · Żółć that must wrap and stay inside this window.".into(), true, window, cx);
            });
            let tip = bounds(cx, window, "help-popup");
            assert!(
                tip.left() >= px(4.)
                    && tip.right() <= px(width - 4.)
                    && tip.top() >= px(4.)
                    && tip.bottom() <= px(height - 4.)
            );
        }
    }
}

#[gpui_kit::test]
fn empty_credentials_report_the_problem_without_sending_a_secret(cx: &mut TestAppContext) {
    let (window, _, mut received, _) = open(cx, State::default());
    click(cx, window, "settings");
    click(cx, window, "session-token");
    assert!(received.try_recv().is_err());
    assert!(present(cx, window, "token-error"));
    assert!(present(cx, window, "bot-token"));
    // The focus returns to the still-masked native input for correction.
    act(cx, window, |window, _| {
        assert!(window.try_find("bot-token").is_some());
    });
}

#[gpui_kit::test]
fn keyboard_reveals_controls_below_the_settings_fold(cx: &mut TestAppContext) {
    let ((window, _, _, _), _) = open_at(cx, State::default(), 420., 480.);
    click(cx, window, "settings");
    click(cx, window, "setup-help");
    act(cx, window, |window, cx| {
        for _ in 0..20 {
            if window.find("developer-portal").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find("developer-portal").focused(), Some(true));
    });
    let target = bounds(cx, window, "developer-portal");
    assert!(
        target.top() >= px(60.) && target.bottom() <= px(464.),
        "focused setup help must scroll into view: {target:?}"
    );
}

#[gpui_kit::test]
fn keyboard_cannot_commit_disabled_channels_or_change_the_current_destination(
    cx: &mut TestAppContext,
) {
    let (window, view, mut received, _) = open(cx, configured());
    click(cx, window, "settings");
    click(cx, window, "channel-selector");
    act(cx, window, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
    });
    cx.run_until_parked();
    assert!(received.try_recv().is_err());
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).channel_select.read(cx).selected_value(),
            Some(&3)
        );
    });
    assert!(present(cx, window, "active-destination"));
}

#[gpui_kit::test]
fn authentication_advances_to_destination_without_starting_playback(cx: &mut TestAppContext) {
    let (window, view, mut received, updates) = open(cx, State::default());
    click(cx, window, "settings");
    let mut authenticated = configured();
    authenticated.setup.destination = None;
    authenticated.setup.status = Status::NeedsDestination;
    authenticated.available = false;
    updates.send_replace(authenticated);
    cx.run_until_parked();
    assert!(present(cx, window, "server-selector"));
    assert!(!present(cx, window, "bot-token"));
    assert!(
        received.try_recv().is_err(),
        "authentication alone cannot join voice or play"
    );
    act(cx, window, |window, cx| {
        assert!(
            view.read(cx)
                .server_select
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
}

#[gpui_kit::test]
fn long_disabled_channel_names_keep_their_reason_inside_the_option(cx: &mut TestAppContext) {
    let mut snapshot = configured();
    snapshot.setup.channels[2].name =
        "Private listening room · 夜の音楽 · A very long unavailable channel name".into();
    let ((window, _, _, _), _) = open_at(cx, snapshot, 420., 480.);
    click(cx, window, "settings");
    click(cx, window, "channel-selector");
    let row = bounds(cx, window, ("destination-choice", 7_u64));
    let reason = bounds(cx, window, ("destination-reason", 7_u64));
    assert!(
        reason.top() >= row.top() && reason.bottom() <= row.bottom(),
        "disabled reason must fit its own option: {reason:?}, row {row:?}"
    );
}

#[gpui_kit::test]
fn rejected_saved_credentials_can_still_be_removed(cx: &mut TestAppContext) {
    let mut snapshot = State::default();
    snapshot.setup.status =
        Status::Failed("Discord rejected the saved bot token. Enter a new bot token.".into());
    let (window, _, _, _) = open(cx, snapshot);
    click(cx, window, "settings");
    assert!(
        present(cx, window, "remove-token"),
        "failed authentication must not strand saved credentials"
    );
}
