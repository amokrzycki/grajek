use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext, Context, Entity, Focusable, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, Task, Window, div,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

use crate::player::{Command, Phase, State};

pub struct PlayerApp {
    input: Entity<InputState>,
    commands: mpsc::Sender<Command>,
    state: State,
    /// A Pause/Resume request is in flight; cleared by the controller's next state.
    control_pending: bool,
    _input_subscription: Subscription,
    _updates: Task<()>,
}

fn clock(position: Duration) -> String {
    let seconds = position.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// `01:24 / 03:45`, or `01:24 (duration unavailable)` when yt-dlp gave no duration.
fn format_progress(position: Duration, duration: Option<Duration>) -> String {
    match duration {
        Some(duration) => format!("{} / {}", clock(position), clock(duration)),
        None => format!("{} (duration unavailable)", clock(position)),
    }
}

impl PlayerApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        commands: mpsc::Sender<Command>,
        mut states: watch::Receiver<State>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste YouTube URL…"));
        window.focus(&input.read(cx).focus_handle(cx), cx);
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.play(cx);
            }
        });
        let state = states.borrow_and_update().clone();
        let updates = cx.spawn(async move |entity, cx| {
            while states.changed().await.is_ok() {
                let state = states.borrow_and_update().clone();
                if entity
                    .update(cx, |this, cx| {
                        this.state = state;
                        this.control_pending = false;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            input,
            commands,
            state,
            control_pending: false,
            _input_subscription: subscription,
            _updates: updates,
        }
    }

    fn play(&mut self, cx: &mut Context<Self>) {
        if !self.state.can_play() {
            return;
        }
        let url = self.input.read(cx).value().to_string();
        if self.commands.try_send(Command::Play(url)).is_ok() {
            // Close the gap before the controller's first update arrives.
            self.state.phase = Phase::Connecting;
            self.state.error = None;
            self.state.track = None;
            self.control_pending = false;
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.state.can_stop() && self.commands.try_send(Command::Stop).is_ok() {
            self.state.phase = Phase::Stopping;
            self.state.track = None;
            self.control_pending = false;
            cx.notify();
        }
    }

    /// Sends Pause or Resume without guessing the outcome: the button keeps its label
    /// (and is disabled against repeat clicks) until the controller reports the real state.
    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.control_pending {
            return;
        }
        let command = if self.state.can_pause() {
            Command::Pause
        } else if self.state.can_resume() {
            Command::Resume
        } else {
            return;
        };
        if self.commands.try_send(command).is_ok() {
            self.control_pending = true;
            cx.notify();
        }
    }
}

fn connection(state: &State) -> &'static str {
    if state.available {
        "Discord connected"
    } else if state.phase == Phase::Initializing {
        "Connecting to Discord…"
    } else {
        "Discord unavailable"
    }
}

impl PlayerApp {
    fn header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let color = if self.state.available {
            theme.green
        } else if self.state.phase == Phase::Initializing {
            theme.muted_foreground
        } else {
            theme.danger
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Grajek"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_xs()
                    .text_color(color)
                    .child("●")
                    .child(connection(&self.state)),
            )
    }

    fn url_row(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .gap_2()
            .child(Input::new(&self.input).id("url").flex_1().min_w_0())
            .child(
                Button::new("play")
                    .primary()
                    .label("Play URL")
                    .disabled(!self.state.can_play())
                    .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
            )
    }

    /// Title and status are single lines that truncate with an ellipsis, so a long
    /// title can never widen the window or push the controls away.
    fn now_playing(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let phase_color = if self.state.error.is_some() {
            theme.danger
        } else if self.state.phase == Phase::Playing {
            theme.green
        } else if self.state.phase == Phase::Paused {
            theme.yellow
        } else {
            theme.muted_foreground
        };
        let track = self.state.visible_track();
        let (title, title_color) = match track {
            Some(info) => (
                info.title
                    .clone()
                    .unwrap_or_else(|| "Untitled track".into()),
                theme.foreground,
            ),
            None => ("Nothing playing".into(), theme.muted_foreground),
        };
        let progress = track.map(|info| format_progress(self.state.position, info.duration));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_3()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Now playing"),
            )
            .child(
                div()
                    .id("title")
                    .w_full()
                    .truncate()
                    .text_base()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(title_color)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_none()
                            .text_color(phase_color)
                            .child("●")
                            .child(self.state.phase.label()),
                    )
                    .children(progress.map(|progress| {
                        div()
                            .id("progress")
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(progress)
                    })),
            )
    }

    fn controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let label = if self.state.phase == Phase::Paused {
            "Resume"
        } else {
            "Pause"
        };
        let toggle_enabled =
            !self.control_pending && (self.state.can_pause() || self.state.can_resume());
        div()
            .flex()
            .gap_2()
            .child(
                Button::new("pause-resume")
                    .label(label)
                    .flex_1()
                    .disabled(!toggle_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pause(cx))),
            )
            .child(
                Button::new("stop")
                    .label("Stop")
                    .w_24()
                    .disabled(!self.state.can_stop())
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            )
    }
}

impl Render for PlayerApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.header(cx))
            .child(self.url_row(cx))
            .child(self.now_playing(cx))
            .child(self.controls(cx))
            // Errors wrap and clamp to four lines instead of stretching the window.
            .children(self.state.error.clone().map(|error| {
                div()
                    .id("error")
                    .w_full()
                    .text_xs()
                    .text_color(theme.danger)
                    .line_clamp(4)
                    .child(error)
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Phase, PlayerApp, State, clock, format_progress};
    use crate::media::TrackInfo;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, ClipboardItem, Entity, TestAppContext, WindowBounds,
        WindowOptions, px, size,
    };
    use std::time::Duration;
    use tokio::sync::{mpsc, watch};

    fn state(phase: Phase) -> State {
        State {
            phase,
            available: true,
            ..State::default()
        }
    }

    fn playing(phase: Phase, title: Option<&str>, seconds: u64) -> State {
        State {
            track: Some(TrackInfo {
                title: title.map(Into::into),
                duration: Some(Duration::from_secs(225)),
            }),
            position: Duration::from_secs(seconds),
            ..state(phase)
        }
    }

    type Harness = (
        AnyWindowHandle,
        Entity<PlayerApp>,
        mpsc::Receiver<Command>,
        watch::Sender<State>,
    );

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn open(cx: &mut TestAppContext, initial: State) -> Harness {
        let (commands, received) = mpsc::channel(4);
        let (states, updates) = watch::channel(initial);
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(460.), px(320.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| PlayerApp::new(window, cx, commands, updates)),
            )
            .expect("test window should open")
        });
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("test window should render");
        (window, view, received, states)
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .expect("test window should dispatch the click");
    }

    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn button(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        id: &'static str,
    ) -> (String, bool) {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let snapshot = window.find(id);
            (
                snapshot.label().unwrap_or_default().to_owned(),
                // GPUI Kit's Button reports no `disabled` flag; a disabled button is not focusable.
                snapshot
                    .disabled()
                    .unwrap_or_else(|| snapshot.focused().is_none()),
            )
        })
        .expect("test window should expose the button")
    }

    #[test]
    fn position_is_formatted_as_minutes_and_seconds() {
        assert_eq!(clock(Duration::ZERO), "00:00");
        assert_eq!(clock(Duration::from_millis(59_999)), "00:59");
        assert_eq!(clock(Duration::from_secs(84)), "01:24");
        assert_eq!(clock(Duration::from_secs(75 * 60 + 3)), "75:03");
        assert_eq!(
            format_progress(Duration::from_secs(84), Some(Duration::from_secs(225))),
            "01:24 / 03:45"
        );
        assert_eq!(
            format_progress(Duration::from_secs(84), None),
            "01:24 (duration unavailable)"
        );
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn paste_enter_duplicate_play_and_stop(cx: &mut TestAppContext) {
        let (window, view, mut received, states) = open(cx, state(Phase::Ready));
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                "https://youtu.be/aqz-KE-bpKQ".into(),
            ));
            window.press("ctrl-v", cx);
        })
        .expect("test window should accept clipboard paste");
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.press("enter", cx);
            window.press("enter", cx);
            window.click("play", cx);
        })
        .expect("test window should dispatch Enter and Play");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/aqz-KE-bpKQ")
        );
        assert!(
            received.try_recv().is_err(),
            "busy UI must reject duplicate Play"
        );
        click(cx, window, "stop");
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        states.send_replace(state(Phase::Ready));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            window.press("ctrl-a", cx);
            window.input("https://music.youtube.com/watch?v=BaW_jenozKc", cx);
            window.click("play", cx);
        })
        .expect("test window should accept a replacement URL and Play");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url.contains("music.youtube.com"))
        );
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Connecting));
    }

    #[gpui_kit::test]
    fn pause_resume_button_follows_controller_state(cx: &mut TestAppContext) {
        let (window, _view, mut received, states) =
            open(cx, playing(Phase::Playing, Some("Song"), 84));
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        // Rapid repeat clicks are ignored until the controller answers.
        click(cx, window, "pause-resume");
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), true));
        states.send_replace(playing(Phase::Paused, Some("Song"), 85));
        cx.run_until_parked();
        assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Resume)));
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
        states.send_replace(playing(Phase::Playing, Some("Song"), 85));
        cx.run_until_parked();
        assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
    }

    #[gpui_kit::test]
    fn failed_pause_never_leaves_a_false_paused_state(cx: &mut TestAppContext) {
        let (window, view, mut received, states) =
            open(cx, playing(Phase::Playing, Some("Song"), 12));
        click(cx, window, "pause-resume");
        assert!(matches!(received.try_recv(), Ok(Command::Pause)));
        // The GUI did not guess Paused while waiting.
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Playing));
        states.send_replace(State {
            error: Some("Could not pause playback. The track is no longer active.".into()),
            ..state(Phase::Error)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(app.state.phase, Phase::Error);
            assert!(app.state.visible_track().is_none());
            assert!(!app.control_pending);
        });
        assert!(button(cx, window, "pause-resume").1, "no track to pause");
        click(cx, window, "pause-resume");
        assert!(received.try_recv().is_err());
    }

    #[gpui_kit::test]
    fn stop_works_while_paused(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) =
            open(cx, playing(Phase::Paused, Some("Song"), 30));
        assert!(!button(cx, window, "stop").1);
        click(cx, window, "stop");
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Stopping));
        assert!(button(cx, window, "pause-resume").1);
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn new_url_replaces_a_paused_track(cx: &mut TestAppContext) {
        let (window, view, mut received, _states) =
            open(cx, playing(Phase::Paused, Some("Old"), 30));
        cx.update_window(window, |_, window, cx| {
            window.click("url", cx);
            window.input("https://youtu.be/BaW_jenozKc", cx);
            window.click("play", cx);
        })
        .expect("test window should accept a URL while paused");
        assert!(
            matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/BaW_jenozKc")
        );
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(app.state.phase, Phase::Connecting);
            assert!(app.state.visible_track().is_none() && app.state.track.is_none());
        });
        assert!(button(cx, window, "pause-resume").1);
    }

    #[gpui_kit::test]
    fn missing_title_duration_and_long_titles_render(cx: &mut TestAppContext) {
        let (window, view, _received, states) = open(cx, playing(Phase::Playing, None, 5));
        let long = "Very long title ".repeat(80);
        for track in [
            TrackInfo {
                title: None,
                duration: None,
            },
            TrackInfo {
                title: Some(long.clone()),
                duration: Some(Duration::from_hours(3)),
            },
            TrackInfo {
                title: Some("ż".repeat(5000)),
                duration: None,
            },
        ] {
            states.send_replace(State {
                track: Some(track),
                error: Some(long.clone()),
                ..playing(Phase::Paused, None, 3725)
            });
            cx.run_until_parked();
            cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
            // Layout must survive the frame; the window never grows or panics.
            assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
        }
    }

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn unavailable_configuration_disables_actions_but_keeps_input_usable(cx: &mut TestAppContext) {
        let (commands, mut received) = mpsc::channel(4);
        let (_, updates) = watch::channel(State {
            phase: Phase::Error,
            error: Some("Set DISCORD_BOT_TOKEN in .env, then restart.".into()),
            available: false,
            ..State::default()
        });
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| PlayerApp::new(window, cx, commands, updates))
            })
            .expect("test window should open")
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("url", cx);
            window.input("https://youtu.be/aqz-KE-bpKQ", cx);
            window.press("enter", cx);
            window.click("play", cx);
            window.click("pause-resume", cx);
            window.click("stop", cx);
        })
        .expect("test window should accept input and disabled actions");
        assert!(received.try_recv().is_err());
        cx.update(|cx| {
            assert_eq!(
                view.read(cx).input.read(cx).value(),
                "https://youtu.be/aqz-KE-bpKQ"
            );
        });
    }
}
