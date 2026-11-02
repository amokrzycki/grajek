//! One help lifecycle for the window; menus carry their own decision-making copy.
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui_kit::base::{ElementExt, TooltipDefaults, TooltipOverlay, TooltipRequest};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled,
    TestSupportExt, Window, div, px,
};

type ActiveHelp = Option<(Bounds<Pixels>, bool)>;

#[derive(Clone)]
pub(super) struct Help {
    pub overlay: Entity<TooltipOverlay>,
    enabled: Rc<Cell<bool>>,
    active: Rc<Cell<ActiveHelp>>,
}

impl Help {
    pub fn new(cx: &mut App) -> Self {
        TooltipDefaults::new()
            .with_show_delay(Duration::from_millis(500))
            .with_grace_period(Duration::from_millis(100))
            .install(cx);
        Self {
            overlay: cx.new(|_| TooltipOverlay::new()),
            enabled: Rc::new(Cell::new(true)),
            active: Rc::new(Cell::new(None)),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.active.set(None);
        self.overlay.update(cx, TooltipOverlay::hide);
    }

    pub fn enable(&self, enabled: bool, cx: &mut App) {
        if self.enabled.replace(enabled) != enabled {
            self.dismiss(cx);
        }
    }

    pub(super) fn show(
        &self,
        bounds: Bounds<Pixels>,
        text: SharedString,
        keyboard: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.enabled.get() {
            return;
        }
        self.active.set(Some((bounds, keyboard)));
        self.overlay.update(cx, |overlay, cx| {
            let request = TooltipRequest::new(bounds, move |_, cx| {
                cx.new(|_| HelpText(text.clone())).into()
            });
            overlay.request_show(
                if keyboard {
                    request.with_show_delay(Duration::ZERO)
                } else {
                    request
                },
                window,
                cx,
            );
        });
    }
}

pub(super) trait HelpExt: ParentElement + InteractiveElement + Styled + Sized {
    fn help(self, text: impl Into<SharedString>, help: &Help) -> Self {
        let text = text.into();
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let measured = bounds.clone();
        let hovered = bounds.clone();
        let pointer_help = help.clone();
        let pointer_text = text.clone();
        let keyboard_help = help.clone();
        let mut element = self
            .relative()
            .on_prepaint(move |rect, _, _| measured.set(rect));
        element.interactivity().on_hover(move |hover, window, cx| {
            if *hover {
                pointer_help.show(hovered.get(), pointer_text.clone(), false, window, cx);
            } else if pointer_help.active.get() == Some((hovered.get(), false)) {
                pointer_help
                    .overlay
                    .update(cx, |overlay, cx| overlay.request_hide(window, cx));
            }
        });
        element.on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "f1" {
                keyboard_help.show(bounds.get(), text.clone(), true, window, cx);
                cx.stop_propagation();
            }
        })
    }
}

impl<T: ParentElement + InteractiveElement + Styled> HelpExt for T {}

struct HelpText(SharedString);

impl Render for HelpText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("help-popup")
            .role(gpui_kit::Role::Tooltip)
            .aria_label(self.0.clone())
            .w(px(300.).min(window.viewport_size().width - px(32.)))
            .max_h(window.viewport_size().height - px(32.))
            .overflow_y_scroll()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .font_family(theme.font_family.clone())
            .text_sm()
            .line_height(px(20.))
            .text_color(theme.popover_foreground)
            .child(self.0.clone())
            .test_support()
    }
}
