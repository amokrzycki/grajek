use super::health::{Health, sort_servers};
use super::session::{Discord, channel_reason};
use super::voice::{VoiceEvents, retryable};
use crate::discord::setup::{Destination, Server, Setup, Status};
use anyhow::Result;
use serenity::all::{
    ChannelId, ChannelType, GatewayIntents, GuildId, Permissions, ShardManager, UserId,
};
use songbird::{Songbird, events::context_data::DisconnectReason, model::CloseCode};
use std::sync::Arc;
use tokio::sync::watch;

#[test]
fn channels_require_all_permissions_and_never_allow_stage() {
    let required = Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK;
    assert!(channel_reason(ChannelType::Voice, required).is_none());
    for missing in [
        Permissions::VIEW_CHANNEL,
        Permissions::CONNECT,
        Permissions::SPEAK,
    ] {
        assert!(channel_reason(ChannelType::Voice, required - missing).is_some());
    }
    assert!(channel_reason(ChannelType::Stage, Permissions::all()).is_some());
}

#[test]
fn guild_sort_is_deterministic_with_unicode_and_duplicate_names() {
    let mut servers = vec![
        Server {
            id: GuildId::new(3),
            name: "żółć".into(),
            available: false,
        },
        Server {
            id: GuildId::new(2),
            name: "Alpha".into(),
            available: true,
        },
        Server {
            id: GuildId::new(1),
            name: "alpha".into(),
            available: true,
        },
    ];
    sort_servers(&mut servers);
    assert_eq!(
        servers
            .iter()
            .map(|server| server.id.get())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

fn selected_health() -> Health {
    Health {
        setup: Setup {
            destination: Some(Destination {
                bot: UserId::new(3),
                guild: GuildId::new(1),
                channel: ChannelId::new(2),
                guild_name: "Server".into(),
                channel_name: "Voice".into(),
            }),
            servers: vec![Server {
                id: GuildId::new(1),
                name: "Server".into(),
                available: true,
            }],
            ..Setup::default()
        },
        voice_requested: true,
        voice_epoch: 4,
        ..Health::default()
    }
}

#[test]
fn temporary_guild_outage_keeps_selection_and_return_does_not_rejoin() {
    let mut health = selected_health();
    let selected = health.setup.destination.clone();
    health.guild_lost(GuildId::new(1), true);
    assert_eq!(health.setup.destination, selected);
    assert!(!health.setup.servers[0].available && !health.voice_requested);
    assert_eq!(health.voice_epoch, 5);
    health.guild_available(Server {
        id: GuildId::new(1),
        name: "Server".into(),
        available: true,
    });
    assert_eq!(health.setup.destination, selected);
    assert!(health.setup.servers[0].available);
    assert!(!health.voice && !health.voice_requested && !health.voice_retry);
}

#[test]
fn ready_reconciles_missed_removal_and_invalidates_discovery_results() {
    let mut health = selected_health();
    let before = health.discovery_epoch;
    health.reconcile_guilds(&[]);
    assert!(health.setup.destination.is_none());
    assert!(health.discovery_epoch > before);
    assert!(!health.voice_requested && !health.voice_retry);
    assert_eq!(health.setup.servers.len(), 0);
}

#[test]
fn rest_refresh_removal_requests_disconnect_and_invalidates_selection() {
    let mut health = selected_health();
    health.setup.status = Status::Ready;
    health.voice = true;
    health.voice_retry = true;
    let epoch = health.discovery_epoch;
    assert!(health.refresh_servers(epoch, Vec::new()));
    assert!(health.setup.destination.is_none());
    assert!(matches!(health.setup.status, Status::NeedsDestination));
    assert!(!health.voice_requested && !health.voice_retry);
    assert_eq!(health.voice_epoch, 5);
    // Keep confirmed voice state until disconnect succeeds so failure remains retryable.
    assert!(health.voice);
    assert!(!health.refresh_servers(epoch, Vec::new()));
}

#[test]
fn stale_rest_refresh_cannot_remove_current_destination() {
    let mut health = selected_health();
    health.discovery_epoch = 3;
    let selected = health.setup.destination.clone();
    assert!(!health.refresh_servers(2, Vec::new()));
    assert_eq!(health.setup.destination, selected);
    assert!(health.voice_requested);
}

fn events() -> (VoiceEvents, watch::Receiver<Health>) {
    let (health, receiver) = watch::channel(Health {
        gateway: true,
        voice: true,
        voice_epoch: 2,
        voice_requested: true,
        voice_session: Some("current".into()),
        ..Health::default()
    });
    (VoiceEvents { health, epoch: 2 }, receiver)
}

#[tokio::test]
async fn cancelled_idle_disconnect_requires_a_fresh_connection() -> Result<()> {
    use std::{future::Future as _, task::Poll};

    let manager = Songbird::serenity();
    manager.initialise_client_data(1, UserId::new(3));
    let guild = GuildId::new(1);
    let channel = ChannelId::new(2);
    let call = manager.get_or_insert(guild);
    // Songbird buffers this gateway request without opening a network connection.
    drop(call.lock().await.join_gateway(channel).await?);
    let (events, health) = events();
    // Permission checks fail locally, so only a valid existing connection can succeed.
    let http = Arc::new(
        serenity::http::HttpBuilder::new("unused-test-token")
            .proxy("invalid proxy")
            .build(),
    );
    let (shards, _) = ShardManager::new(serenity::gateway::ShardManagerOptions {
        data: Arc::new(tokio::sync::RwLock::new(serenity::prelude::TypeMap::new())),
        event_handlers: vec![],
        raw_event_handlers: vec![],
        shard_index: 0,
        shard_init: 0,
        shard_total: 1,
        voice_manager: None,
        ws_url: Arc::new(tokio::sync::Mutex::new(String::new())),
        http: http.clone(),
        intents: GatewayIntents::empty(),
        presence: None,
    });
    let discord = Discord {
        manager,
        guild: Some(guild),
        channel: Some(channel),
        user: UserId::new(3),
        http,
        health,
        health_tx: events.health,
        shards,
        gateway: tokio::spawn(std::future::pending()),
    };
    assert_eq!(
        call.lock().await.current_channel().map(|id| id.0.get()),
        Some(channel.get())
    );
    discord.connect().await?;

    let guard = call.lock().await;
    let mut leave = Box::pin(discord.disconnect());
    assert!(std::future::poll_fn(|cx| Poll::Ready(leave.as_mut().poll(cx).is_pending())).await);
    // Play cancels idle teardown while Songbird's call lock is held.
    drop(leave);
    drop(guard);
    assert!(discord.health.borrow().voice);
    assert!(!discord.health.borrow().voice_requested);
    assert!(discord.connect().await.is_err());
    Ok(())
}

#[test]
fn only_transient_disconnect_reasons_allow_rejoin() {
    for reason in [
        DisconnectReason::Io,
        DisconnectReason::TimedOut,
        DisconnectReason::WsClosed(Some(CloseCode::SessionInvalid)),
        DisconnectReason::WsClosed(Some(CloseCode::SessionTimeout)),
        DisconnectReason::WsClosed(Some(CloseCode::VoiceServerCrash)),
    ] {
        assert!(retryable(Some(reason)));
    }
    for reason in [
        None,
        Some(DisconnectReason::Requested),
        Some(DisconnectReason::AttemptDiscarded),
        Some(DisconnectReason::Internal),
        Some(DisconnectReason::ProtocolViolation),
        Some(DisconnectReason::WsClosed(None)),
        Some(DisconnectReason::WsClosed(Some(CloseCode::Disconnected))),
        Some(DisconnectReason::WsClosed(Some(
            CloseCode::AuthenticationFailed,
        ))),
        Some(DisconnectReason::WsClosed(Some(CloseCode::RateLimited))),
        Some(DisconnectReason::WsClosed(Some(CloseCode::CallTerminated))),
    ] {
        assert!(!retryable(reason));
    }
}

#[test]
fn stale_and_requested_driver_events_cannot_damage_a_new_connection() {
    let (events, health) = events();
    events.update("old", false, false, Some(DisconnectReason::Io));
    assert!(health.borrow().voice);
    for reason in [
        None,
        Some(DisconnectReason::Requested),
        Some(DisconnectReason::AttemptDiscarded),
    ] {
        events.update("current", false, false, reason);
        assert!(health.borrow().voice && health.borrow().error().is_none());
    }
    let old = VoiceEvents {
        health: events.health,
        epoch: 1,
    };
    old.update("old", false, false, Some(DisconnectReason::Io));
    old.update("old", true, false, None);
    assert_eq!(health.borrow().voice_session.as_deref(), Some("current"));
    assert!(health.borrow().voice && health.borrow().error().is_none());
}

#[test]
fn intentional_disconnect_ignores_late_driver_callbacks() {
    let (events, health) = events();
    events.health.send_modify(|health| {
        health.voice_requested = false;
        health.voice = false;
    });
    events.update("current", false, false, Some(DisconnectReason::Io));
    events.update("current", true, true, None);
    assert!(!health.borrow().voice);
    assert!(health.borrow().error().is_none());
    assert!(!health.borrow().voice_retry);
}

#[test]
fn internal_reconnect_reports_interruption_without_requesting_another_join() {
    let (events, health) = events();
    events.update("current", true, true, None);
    assert!(health.borrow().voice);
    assert!(!health.borrow().voice_retry);
    assert!(
        health
            .borrow()
            .error()
            .is_some_and(|e| e.contains("interrupted"))
    );
}
#[test]
fn a_kick_cannot_be_overridden_by_a_late_driver_event() {
    let (events, health) = events();
    events
        .health
        .send_if_modified(|health| health.voice_state("current", false));
    events.update("current", false, false, Some(DisconnectReason::Io));
    events.update("current", true, true, None);
    assert!(!health.borrow().voice && !health.borrow().voice_retry);
    assert!(
        health
            .borrow()
            .error()
            .is_some_and(|error| error.contains("left or was moved"))
    );
    // A gateway leave for an older session must not affect a newer connection.
    let (events, health) = self::events();
    events
        .health
        .send_if_modified(|health| health.voice_state("old", false));
    assert!(health.borrow().voice && health.borrow().error().is_none());
}

#[test]
fn a_kick_during_join_invalidates_a_late_driver_connect() {
    let (sender, health) = watch::channel(Health {
        gateway: true,
        voice_requested: true,
        voice_epoch: 2,
        ..Health::default()
    });
    let events = VoiceEvents {
        health: sender.clone(),
        epoch: 2,
    };
    sender.send_if_modified(|health| health.voice_state("current", true));
    assert!(!health.borrow().voice);
    // A delayed leave for the previous session must not cancel this join.
    sender.send_if_modified(|health| health.voice_state("old", false));
    assert!(health.borrow().voice_requested);
    assert!(health.borrow().error().is_none());
    sender.send_if_modified(|health| health.voice_state("current", false));
    sender.send_if_modified(|health| health.voice_state("current", true));
    events.update("current", true, false, None);
    assert!(!health.borrow().voice);
    assert!(!health.borrow().voice_requested);
    assert!(!health.borrow().voice_retry);
    assert!(health.borrow().error().is_some());
}
