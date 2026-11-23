//! Window actions are queued for the GPUI thread; tray code never touches the window itself.
use crate::{platform::media, player};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;

mod icon;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    Show,
    Hide,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Status,
    Show,
    Hide,
    PlayPause,
    Next,
    Stop,
    LeaveVoice,
    Quit,
}

impl MenuItem {
    pub fn id(self) -> i32 {
        match self {
            Self::Status => 8,
            Self::Show => 1,
            Self::Hide => 2,
            Self::PlayPause => 3,
            Self::Next => 4,
            Self::Stop => 5,
            Self::LeaveVoice => 6,
            Self::Quit => 7,
        }
    }

    pub fn from_id(id: i32) -> Option<Self> {
        [
            Self::Status,
            Self::Show,
            Self::Hide,
            Self::PlayPause,
            Self::Next,
            Self::Stop,
            Self::LeaveVoice,
            Self::Quit,
        ]
        .into_iter()
        .find(|item| item.id() == id)
    }
}

#[derive(Clone, Default)]
pub struct TrayAvailability(Arc<AtomicBool>);

impl TrayAvailability {
    pub fn mark_available(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn mark_unavailable(&self) {
        self.0.store(false, Ordering::Release);
    }

    #[cfg(target_os = "linux")]
    pub fn take_available(&self) -> bool {
        self.0.swap(false, Ordering::AcqRel)
    }

    pub fn is_available(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
pub struct Endpoint {
    pub player: media::Endpoint,
    pub window: mpsc::Sender<WindowAction>,
    pub available: TrayAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
}

impl Endpoint {
    pub fn items(&self) -> Vec<Item> {
        menu(&self.player.states.borrow())
    }

    pub fn activate(&self, id: i32) {
        if !self.available() {
            return;
        }
        if !self
            .items()
            .iter()
            .any(|item| item.id == id && item.enabled)
        {
            return;
        }
        let Some(item) = MenuItem::from_id(id) else {
            return;
        };
        match item {
            MenuItem::Show => {
                let _ = self.window.try_send(WindowAction::Show);
            }
            MenuItem::Hide => {
                let _ = self.window.try_send(WindowAction::Hide);
            }
            MenuItem::PlayPause => {
                let _ = self.player.send(player::MediaCommand::PlayPause);
            }
            MenuItem::Next => {
                let _ = self.player.send(player::MediaCommand::Next);
            }
            MenuItem::Stop => {
                let _ = self.player.send(player::MediaCommand::Stop);
            }
            MenuItem::LeaveVoice => {
                let _ = self.player.commands.try_send(player::Command::LeaveVoice);
            }
            MenuItem::Quit => {
                let _ = self.window.try_send(WindowAction::Quit);
            }
            MenuItem::Status => {}
        }
    }

    pub fn hide_on_close(&self) -> bool {
        self.available() && self.player.states.borrow().close_to_tray
    }

    pub fn available(&self) -> bool {
        self.available.is_available()
    }
}

pub fn menu(state: &player::State) -> Vec<Item> {
    let snapshot = media::Snapshot::from(state);
    let destination = state.setup.destination.as_ref().map_or_else(
        || "Choose a Discord destination".into(),
        |destination| format!("{} / {}", destination.guild_name, destination.channel_name),
    );
    [
        (
            MenuItem::Status,
            format!("{} · {destination}", state.phase.label()),
            false,
        ),
        (MenuItem::Show, "Show Grajek".into(), true),
        (MenuItem::Hide, "Hide Grajek".into(), true),
        (
            MenuItem::PlayPause,
            if snapshot.can_pause {
                "Pause"
            } else if state.phase == player::Phase::Paused {
                "Resume"
            } else {
                "Play Queue"
            }
            .into(),
            snapshot.play_enabled(),
        ),
        (MenuItem::Next, state.next_label().into(), snapshot.can_next),
        (MenuItem::Stop, "Stop".into(), snapshot.can_stop),
        (
            MenuItem::LeaveVoice,
            "Leave Voice".into(),
            state.can_leave_voice(),
        ),
        (MenuItem::Quit, "Quit Grajek".into(), true),
    ]
    .into_iter()
    .map(|(menu_item, label, enabled)| Item {
        id: menu_item.id(),
        label,
        enabled,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unconfigured_menu_has_no_playback_actions() {
        let items = menu(&player::State::default());
        assert!(
            items
                .iter()
                .filter(
                    |item| (MenuItem::PlayPause.id()..=MenuItem::LeaveVoice.id())
                        .contains(&item.id)
                )
                .all(|item| !item.enabled)
        );
        assert!(
            items
                .iter()
                .any(|item| item.id == MenuItem::Quit.id() && item.enabled)
        );
    }
    #[test]
    fn paused_menu_uses_resume_and_position_does_not_change_menu() {
        let mut state = player::State {
            phase: player::Phase::Paused,
            available: true,
            ..Default::default()
        };
        let before = menu(&state);
        assert_eq!(before[3].label, "Resume");
        state.position = std::time::Duration::from_secs(42);
        assert_eq!(menu(&state), before);
    }

    fn endpoint(
        state: player::State,
    ) -> (
        Endpoint,
        mpsc::Receiver<WindowAction>,
        mpsc::Receiver<player::Command>,
    ) {
        let (commands, player_rx) = mpsc::channel(16);
        let (_, states) = tokio::sync::watch::channel(state);
        let (window, window_rx) = mpsc::channel(16);
        let available = TrayAvailability::default();
        available.mark_available();
        (
            Endpoint {
                player: media::Endpoint { commands, states },
                window,
                available,
            },
            window_rx,
            player_rx,
        )
    }

    #[test]
    fn hide_and_repeated_show_preserve_playback_and_pending_queue() {
        for phase in [
            player::Phase::Playing,
            player::Phase::Paused,
            player::Phase::Initializing,
        ] {
            let state = player::State {
                phase,
                voice_connected: phase != player::Phase::Initializing,
                queue: vec![crate::queue::QueueItem {
                    id: 9,
                    url: "https://youtu.be/pending".into(),
                    title: Some("Pending".into()),
                    title_unavailable: false,
                }],
                ..Default::default()
            };
            let (endpoint, mut windows, mut player) = endpoint(state.clone());
            for _ in 0..3 {
                endpoint.activate(MenuItem::Hide.id());
                endpoint.activate(MenuItem::Show.id());
                endpoint.activate(MenuItem::Show.id());
                assert_eq!(windows.try_recv(), Ok(WindowAction::Hide));
                assert_eq!(windows.try_recv(), Ok(WindowAction::Show));
                assert_eq!(windows.try_recv(), Ok(WindowAction::Show));
                assert!(windows.try_recv().is_err());
            }
            assert!(
                player.try_recv().is_err(),
                "Visibility must never stop, leave or replace playback"
            );
            let after = endpoint.player.states.borrow();
            assert_eq!(after.phase, state.phase);
            assert_eq!(after.voice_connected, state.voice_connected);
            assert_eq!(after.queue, state.queue);
            assert_eq!(menu(&state)[2].label, "Hide Grajek");
        }
    }

    #[test]
    fn close_only_hides_with_both_opt_in_and_a_usable_tray() {
        for available in [false, true] {
            for close_to_tray in [false, true] {
                let (endpoint, _, _) = endpoint(player::State {
                    close_to_tray,
                    ..Default::default()
                });
                if available {
                    endpoint.available.mark_available();
                } else {
                    endpoint.available.mark_unavailable();
                }
                assert_eq!(endpoint.hide_on_close(), available && close_to_tray);
            }
        }
    }

    #[test]
    fn quit_while_hidden_and_late_callbacks_do_not_reenter_the_controller() {
        let (endpoint, mut windows, mut player) = endpoint(player::State::default());
        endpoint.activate(MenuItem::Hide.id());
        endpoint.activate(MenuItem::Quit.id());
        assert_eq!(windows.try_recv(), Ok(WindowAction::Hide));
        assert_eq!(windows.try_recv(), Ok(WindowAction::Quit));
        endpoint.available.mark_unavailable();
        for item in [
            MenuItem::Show,
            MenuItem::Hide,
            MenuItem::PlayPause,
            MenuItem::Next,
            MenuItem::Stop,
            MenuItem::LeaveVoice,
            MenuItem::Quit,
        ] {
            endpoint.activate(item.id());
        }
        assert!(windows.try_recv().is_err());
        assert!(player.try_recv().is_err());
        assert!(!endpoint.hide_on_close());
    }

    #[test]
    fn unavailable_tray_cannot_hide_or_control_playback() {
        let (endpoint, mut windows, mut player) = endpoint(player::State::default());
        endpoint.available.mark_unavailable();
        for item in [
            MenuItem::Show,
            MenuItem::Hide,
            MenuItem::PlayPause,
            MenuItem::Next,
            MenuItem::Stop,
            MenuItem::LeaveVoice,
            MenuItem::Quit,
        ] {
            endpoint.activate(item.id());
        }
        assert!(windows.try_recv().is_err());
        assert!(player.try_recv().is_err());
    }

    #[test]
    fn menu_items_round_trip_their_wire_ids() {
        for item in [
            MenuItem::Status,
            MenuItem::Show,
            MenuItem::Hide,
            MenuItem::PlayPause,
            MenuItem::Next,
            MenuItem::Stop,
            MenuItem::LeaveVoice,
            MenuItem::Quit,
        ] {
            assert_eq!(MenuItem::from_id(item.id()), Some(item));
        }
        assert_eq!(MenuItem::from_id(0), None);
        assert_eq!(MenuItem::from_id(9), None);
        assert_eq!(
            [1, 2, 3, 4, 5, 6, 7, 8],
            [
                MenuItem::Show.id(),
                MenuItem::Hide.id(),
                MenuItem::PlayPause.id(),
                MenuItem::Next.id(),
                MenuItem::Stop.id(),
                MenuItem::LeaveVoice.id(),
                MenuItem::Quit.id(),
                MenuItem::Status.id(),
            ]
        );
    }
}
