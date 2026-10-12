use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    AppContext, Context, Entity, Focusable, FontWeight, IntoElement, ParentElement, Render, Styled,
    Subscription, Task, Window, div,
};
use tokio::sync::{mpsc, watch};

use crate::player::{Command, Phase, State};

pub struct PlayerApp {
    input: Entity<InputState>,
    commands: mpsc::Sender<Command>,
    state: State,
    _input_subscription: Subscription,
    _updates: Task<()>,
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
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.state.can_stop() && self.commands.try_send(Command::Stop).is_ok() {
            self.state.phase = Phase::Stopping;
            cx.notify();
        }
    }
}

impl Render for PlayerApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let color = if self.state.error.is_some() {
            theme.danger
        } else if self.state.phase == Phase::Playing {
            theme.green
        } else {
            theme.muted_foreground
        };
        div()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Discord Player"),
            )
            .child(Input::new(&self.input).id("url").w_full())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("play")
                            .primary()
                            .label("Play")
                            .flex_1()
                            .disabled(!self.state.can_play())
                            .on_click(cx.listener(|this, _, _, cx| this.play(cx))),
                    )
                    .child(
                        Button::new("stop")
                            .label("Stop")
                            .w_24()
                            .disabled(!self.state.can_stop())
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(color)
                    .child("●")
                    .child(self.state.phase.label()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(self.state.error.clone().unwrap_or_default()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Phase, PlayerApp, State};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, ClipboardItem, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use tokio::sync::{mpsc, watch};

    #[gpui_kit::test]
    #[expect(
        clippy::expect_used,
        reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
    )]
    fn paste_enter_duplicate_play_and_stop(cx: &mut TestAppContext) {
        let (commands, mut received) = mpsc::channel(4);
        let (states, updates) = watch::channel(State {
            phase: Phase::Ready,
            error: None,
            available: true,
        });
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(440.), px(260.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| PlayerApp::new(window, cx, commands, updates)),
            )
            .expect("test window should open")
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
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
        cx.update_window(window, |_, window, cx| window.click("stop", cx))
            .expect("test window should dispatch Stop");
        assert!(matches!(received.try_recv(), Ok(Command::Stop)));
        states.send_replace(State {
            phase: Phase::Ready,
            error: None,
            available: true,
        });
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
