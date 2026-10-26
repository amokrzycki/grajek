//! Tray commands use the controller mailbox; window actions stay on the GPUI thread.
use crate::{native, player};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;

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

#[derive(Clone)]
pub struct Endpoint {
    pub player: native::Endpoint,
    pub window: mpsc::Sender<WindowAction>,
    pub available: Arc<AtomicBool>,
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
        match id {
            1 => {
                let _ = self.window.try_send(WindowAction::Show);
            }
            2 => {
                let _ = self.window.try_send(WindowAction::Hide);
            }
            3 => {
                let _ = self.player.send(native::Command::PlayPause);
            }
            4 => {
                let _ = self.player.send(native::Command::Next);
            }
            5 => {
                let _ = self.player.send(native::Command::Stop);
            }
            6 => {
                let _ = self.player.commands.try_send(player::Command::LeaveVoice);
            }
            7 => {
                let _ = self.window.try_send(WindowAction::Quit);
            }
            _ => {}
        }
    }
    pub fn hide_on_close(&self) -> bool {
        self.available() && self.player.states.borrow().close_to_tray
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }
}

pub fn menu(state: &player::State) -> Vec<Item> {
    let media = native::Snapshot::from(state);
    let destination = state.setup.destination.as_ref().map_or_else(
        || "Choose a Discord destination".into(),
        |destination| format!("{} / {}", destination.guild_name, destination.channel_name),
    );
    [
        (8, format!("{} · {destination}", state.phase.label()), false),
        (1, "Show Grajek".into(), true),
        (2, "Hide Grajek".into(), true),
        (
            3,
            if media.can_pause {
                "Pause"
            } else if state.phase == player::Phase::Paused {
                "Resume"
            } else {
                "Play Queue"
            }
            .into(),
            media.can_play || media.can_pause,
        ),
        (4, state.next_label().into(), media.can_next),
        (5, "Stop".into(), media.can_stop),
        (6, "Leave Voice".into(), state.can_leave_voice()),
        (7, "Quit Grajek".into(), true),
    ]
    .into_iter()
    .map(|(id, label, enabled)| Item { id, label, enabled })
    .collect()
}

pub fn icon_argb() -> Vec<u8> {
    let mut data = vec![0; 16 * 16 * 4];
    for y in 2..14 {
        for x in 2..14 {
            if x == 2 || x == 13 || y == 2 || y == 13 || (x == 7 && (6..11).contains(&y)) {
                data[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&[255, 221, 194, 139]);
            }
        }
    }
    data
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
                .filter(|item| (3..=6).contains(&item.id))
                .all(|item| !item.enabled)
        );
        assert!(items.iter().any(|item| item.id == 7 && item.enabled));
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
        (
            Endpoint {
                player: native::Endpoint { commands, states },
                window,
                available: Arc::new(AtomicBool::new(true)),
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
                endpoint.activate(2);
                endpoint.activate(1);
                endpoint.activate(1);
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
                endpoint.available.store(available, Ordering::Release);
                assert_eq!(endpoint.hide_on_close(), available && close_to_tray);
            }
        }
    }

    #[test]
    fn quit_while_hidden_and_late_callbacks_do_not_reenter_the_controller() {
        let (endpoint, mut windows, mut player) = endpoint(player::State::default());
        endpoint.activate(2);
        endpoint.activate(7);
        assert_eq!(windows.try_recv(), Ok(WindowAction::Hide));
        assert_eq!(windows.try_recv(), Ok(WindowAction::Quit));
        endpoint.available.store(false, Ordering::Release);
        for id in 1..=7 {
            endpoint.activate(id);
        }
        assert!(windows.try_recv().is_err());
        assert!(player.try_recv().is_err());
        assert!(!endpoint.hide_on_close());
    }

    #[test]
    fn unavailable_tray_cannot_hide_or_control_playback() {
        let (endpoint, mut windows, mut player) = endpoint(player::State::default());
        endpoint.available.store(false, Ordering::Release);
        for id in 1..=7 {
            endpoint.activate(id);
        }
        assert!(windows.try_recv().is_err());
        assert!(player.try_recv().is_err());
    }
}
