//! Adapt Kit's raw-focus styling to GPUI's existing per-window input modality.
//! Real focus and all native event/activation semantics remain with the controls.
use gpui_kit::component::{ActiveTheme, FocusableExt, button::Button};
use gpui_kit::{ElementId, Window};

#[derive(Clone, Copy, Default)]
pub(super) struct FocusPresentation {
    pub keyboard: bool,
}

impl FocusPresentation {
    pub fn from_window(window: &Window) -> Self {
        Self {
            keyboard: window.last_input_was_keyboard(),
        }
    }

    pub fn editing_border(self, focused: bool, cx: &gpui_kit::App) -> gpui_kit::Hsla {
        if !focused {
            cx.theme().input
        } else if self.keyboard {
            cx.theme().ring
        } else {
            cx.theme().muted_foreground
        }
    }

    pub fn button(self, id: impl Into<ElementId>) -> Button {
        Button::new(id).focus_ring(self.keyboard)
    }
}
