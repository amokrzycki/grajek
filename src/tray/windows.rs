//! Shell notification icon and HMENU live on GPUI's existing Windows message thread.
use super::Endpoint;
use gpui_kit::{App, Task};
use std::sync::atomic::Ordering;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

struct Resources {
    _icon: TrayIcon,
    endpoint: Endpoint,
}
impl Drop for Resources {
    fn drop(&mut self) {
        self.endpoint.available.store(false, Ordering::Release);
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
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
        .with_icon(Icon::from_rgba(rgba, 16, 16)?)
        .with_tooltip("Grajek")
        .build()?;
    let callback = endpoint.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(id) = event
            .id
            .as_ref()
            .strip_prefix("grajek-")
            .and_then(|id| id.parse().ok())
            && callback.available()
        {
            callback.activate(id);
        }
    }));
    endpoint.available.store(true, Ordering::Release);
    let resources = Resources {
        _icon: icon,
        endpoint: endpoint.clone(),
    };
    let mut states = endpoint.player.states.clone();
    Ok(cx.spawn(async move |_| {
        let _resources = resources;
        let mut previous = initial;
        while states.changed().await.is_ok() {
            let current = endpoint.items();
            if current != previous {
                for (item, view) in items.iter().zip(&current) {
                    item.set_text(&view.label);
                    item.set_enabled(view.enabled);
                }
                previous = current;
            }
        }
    }))
}
