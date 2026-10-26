//! Shell notification icon and HMENU live on GPUI's existing Windows message thread.
use super::{
    Endpoint,
    icon::{self, Visual},
};
use gpui_kit::{App, Task};
use std::sync::atomic::Ordering;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

struct Resources {
    icon: TrayIcon,
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
    let mut states = endpoint.player.states.clone();
    let (initial, mut previous_icon) = {
        let state = states.borrow_and_update();
        (super::menu(&state), Visual::from(&*state))
    };
    let menu = Menu::new();
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
    let icon = TrayIconBuilder::new()
        .with_id("grajek")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(tray_icon(previous_icon)?)
        .with_tooltip("Grajek")
        .build()?;
    endpoint.available.store(true, Ordering::Release);
    let resources = Resources {
        icon,
        endpoint: endpoint.clone(),
    };
    let executor = cx.background_executor().clone();
    Ok(cx.spawn(async move |_| {
        let resources = resources;
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
            tokio::select! {
                () = executor.timer(std::time::Duration::from_millis(50)) => {},
                changed = states.changed() => {
                    if changed.is_err() { break; }
                    let (current, current_icon) = {
                        let state = states.borrow_and_update();
                        (super::menu(&state), Visual::from(&*state))
                    };
                    if current_icon != previous_icon {
                        match tray_icon(current_icon).and_then(|icon| {
                            resources.icon.set_icon(Some(icon)).map_err(Into::into)
                        }) {
                            Ok(()) => previous_icon = current_icon,
                            Err(error) => tracing::warn!(%error, "Could not update Windows tray icon"),
                        }
                    }
                    if current != previous {
                        for (item, view) in items.iter().zip(&current) {
                            item.set_text(&view.label);
                            item.set_enabled(view.enabled);
                        }
                        previous = current;
                    }
                }
            }
        }
    }))
}

fn tray_icon(visual: Visual) -> anyhow::Result<Icon> {
    let bitmap = icon::decode(visual.pngs()[3])?;
    let (width, height) = bitmap.dimensions();
    Ok(Icon::from_rgba(bitmap.into_raw(), width, height)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_creates_a_windows_icon_from_embedded_rgba() -> anyhow::Result<()> {
        for visual in [Visual::Idle, Visual::Playing, Visual::Paused, Visual::Error] {
            tray_icon(visual)?;
        }
        Ok(())
    }
}
