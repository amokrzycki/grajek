//! Shell notification icon and HMENU live on GPUI's existing Windows message thread.
use super::Endpoint;
use gpui_kit::{App, Task};
use std::sync::atomic::Ordering;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

struct Resources {
    _icon: TrayIcon,
    endpoint: Endpoint,
}
impl Drop for Resources {
    fn drop(&mut self) {
        self.endpoint.available.store(false, Ordering::Release);
    }
}

pub fn attach(cx: &App, endpoint: Endpoint) -> Option<Task<()>> {
    match create(cx, endpoint) {
        Ok(task) => Some(task),
        Err(error) => {
            tracing::warn!(%error, "Windows tray unavailable; close still quits");
            None
        }
    }
}

fn create(cx: &App, endpoint: Endpoint) -> anyhow::Result<Task<()>> {
    let menu = Menu::new();
    let initial = endpoint.items();
    let items: Vec<_> = initial
        .iter()
        .map(|item| {
            MenuItem::with_id(
                format!("grajek-{}", item.id),
                &item.label,
                item.enabled,
                None,
            )
        })
        .collect();
    for item in &items {
        menu.append(item)?;
    }
    let mut rgba = super::icon_argb();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.rotate_left(1);
    }
    let icon = TrayIconBuilder::new()
        .with_id("grajek")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(Icon::from_rgba(rgba, 16, 16)?)
        .with_tooltip("Grajek")
        .build()?;
    endpoint.available.store(true, Ordering::Release);
    let resources = Resources {
        _icon: icon,
        endpoint: endpoint.clone(),
    };
    let states = endpoint.player.states.clone();
    let executor = cx.background_executor().clone();
    Ok(cx.spawn(async move |_| {
        let _resources = resources;
        let mut previous = initial;
        while states.has_changed().is_ok() && endpoint.available() {
            // The crate's global handlers are write-once and cannot be unregistered.
            // Drain its receivers on GPUI's thread so no callback retains our endpoint.
            for event in MenuEvent::receiver().try_iter() {
                if let Some(id) = event
                    .id
                    .as_ref()
                    .strip_prefix("grajek-")
                    .and_then(|id| id.parse().ok())
                {
                    endpoint.activate(id);
                }
            }
            for event in TrayIconEvent::receiver().try_iter() {
                if event.id().as_ref() == "grajek"
                    && matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        }
                    )
                {
                    endpoint.activate(1);
                }
            }
            let current = endpoint.items();
            if current != previous {
                for (item, view) in items.iter().zip(&current) {
                    item.set_text(&view.label);
                    item.set_enabled(view.enabled);
                }
                previous = current;
            }
            executor.timer(std::time::Duration::from_millis(50)).await;
        }
    }))
}
