//! Signal Ledger palette. Keep the kit's native components and synchronized theme.
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, px, rgb};

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let installed = cx.text_system().all_font_names();
    #[cfg(target_os = "windows")]
    let preferred = ["Segoe UI", "Segoe UI Variable Text", "Arial"];
    #[cfg(not(target_os = "windows"))]
    let preferred = ["Noto Sans", "Inter", "DejaVu Sans"];
    let family = preferred
        .into_iter()
        .find(|name| installed.iter().any(|installed| installed == name));
    Theme::update(cx, |theme| {
        if let Some(family) = family {
            theme.font_family = family.into();
        }
        theme.font_size = px(14.);
        theme.radius = px(4.);
        theme.radius_lg = px(6.);
        theme.shadow = false;

        let canvas = rgb(0x0019_1b18).into();
        let surface = rgb(0x0023_2620).into();
        let hover = rgb(0x0035_3a30).into();
        let pressed = rgb(0x0042_493b).into();
        let text = rgb(0x00ef_ede5).into();
        let muted = rgb(0x00ad_aea3).into();
        let border = rgb(0x0077_7d6d).into();
        let accent = rgb(0x00dd_c28b).into();
        let accent_text = rgb(0x001e_231a).into();
        let danger = rgb(0x00ed_a99b).into();
        let colors = &mut theme.colors;
        colors.background = canvas;
        colors.foreground = text;
        colors.muted = surface;
        colors.muted_foreground = muted;
        colors.border = border;
        colors.accent = hover;
        colors.accent_foreground = text;
        colors.primary = accent;
        colors.primary_foreground = accent_text;
        colors.primary_hover = rgb(0x00ec_d4a4).into();
        colors.primary_active = rgb(0x00c6_a76b).into();
        colors.secondary = surface;
        colors.secondary_foreground = text;
        colors.secondary_hover = hover;
        colors.secondary_active = pressed;
        colors.button = surface;
        colors.button_foreground = text;
        colors.button_hover = hover;
        colors.button_active = pressed;
        colors.button_primary = accent;
        colors.button_primary_foreground = accent_text;
        colors.button_primary_hover = colors.primary_hover;
        colors.button_primary_active = colors.primary_active;
        colors.button_secondary = surface;
        colors.button_secondary_foreground = text;
        colors.button_secondary_hover = hover;
        colors.button_secondary_active = pressed;
        colors.danger = danger;
        colors.danger_foreground = text;
        colors.button_danger = rgb(0x0059_2f2a).into();
        colors.button_danger_foreground = danger;
        colors.button_danger_hover = rgb(0x0074_3c32).into();
        colors.button_danger_active = rgb(0x0048_2721).into();
        colors.green = rgb(0x00ac_c69b).into();
        colors.yellow = accent;
        colors.ring = accent;
        colors.caret = accent;
        colors.input = border;
        colors.selection = pressed;
        colors.list = canvas;
        colors.list_hover = hover;
        colors.list_active = pressed;
        colors.list_active_border = accent;
        colors.tab = canvas;
        colors.tab_bar = canvas;
        colors.tab_foreground = muted;
        colors.tab_active = canvas;
        colors.tab_active_foreground = accent;
        colors.slider_bar = border;
        colors.slider_thumb = accent;
        colors.scrollbar = canvas;
        colors.scrollbar_thumb = border;
        colors.scrollbar_thumb_hover = muted;
        colors.popover = surface;
        colors.popover_foreground = text;
    });
}

/// Separators convey grouping; unlike control boundaries they can stay quiet.
pub fn separator() -> gpui_kit::Hsla {
    rgb(0x003d_4338).into()
}
