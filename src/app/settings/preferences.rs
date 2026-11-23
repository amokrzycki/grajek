use gpui_kit::component::{FocusableExt, Sizable, button::ButtonVariants, switch::Switch};
use gpui_kit::{Context, IntoElement, ParentElement, Styled, div};

use crate::player::Command;

use super::PlayerApp;
use super::detail;
use super::reveal::RevealExt;

impl PlayerApp {
    pub(super) fn preferences_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        div().flex().flex_none().flex_col().gap_2()
            .child(div().flex().child(self.focus.button("preferences-disclosure").ghost().small().label(if self.preferences_open { "Hide preferences" } else { "Preferences…" })
                .on_click(cx.listener(|this, _, _, cx| { this.preferences_open = !this.preferences_open; cx.notify(); })).reveal("preferences-control", &self.settings_scroll)))
            .children(self.reveal.panel("preferences-reveal", self.reveal.preferences, self.preferences_open, || {
                div().flex().flex_col().gap_2()
                    .child(Switch::new("close-to-tray").focus_ring(self.focus.keyboard).label("Keep playing when the window closes").checked(self.state.close_to_tray)
                        .on_change(cx.listener(|this, enabled, _, _| { this.send(Command::CloseToTray(*enabled)); })).reveal("tray-control", &self.settings_scroll))
                    .child(detail("Uses the system tray when available. Choose Quit in the tray to stop Grajek. Without a tray, closing the window quits.", cx))
                    .into_any_element()
            }))
    }
}
