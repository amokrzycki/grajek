use super::{
    Endpoint,
    icon::{self, Visual},
};
use futures_lite::StreamExt as _;
use std::{collections::HashMap, sync::atomic::Ordering};
use tokio::sync::watch;
use zbus::{
    connection::Builder,
    fdo,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedValue, Str},
};

type Pixmap = (i32, i32, Vec<u8>);
struct Item(Endpoint, Vec<Pixmap>);
struct Menu(Endpoint, std::sync::Arc<std::sync::atomic::AtomicU32>);
type Properties = HashMap<String, OwnedValue>;
type Layout = (i32, Properties, Vec<OwnedValue>);

fn pixmaps(visual: Visual) -> zbus::Result<Vec<Pixmap>> {
    visual
        .pngs()
        .into_iter()
        .zip([16, 22, 32, 48])
        .map(|(png, size)| {
            icon::decode(png).map(|bitmap| (size, size, icon::argb(bitmap.into_raw())))
        })
        .collect::<image::ImageResult<_>>()
        .map_err(|error| zbus::Error::Failure(error.to_string()))
}

fn properties(item: &super::Item) -> Properties {
    HashMap::from([
        (
            "label".into(),
            OwnedValue::from(Str::from(item.label.as_str())),
        ),
        ("enabled".into(), OwnedValue::from(item.enabled)),
        ("visible".into(), OwnedValue::from(true)),
    ])
}

#[expect(
    clippy::unused_self,
    reason = "D-Bus interface requires receivers for protocol constants"
)]
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    fn activate(&self, x: i32, y: i32) {
        let _ = (x, y);
        self.0.activate(1);
    }
    fn secondary_activate(&self, x: i32, y: i32) {
        let _ = (x, y);
        self.0.activate(1);
    }
    fn context_menu(&self, x: i32, y: i32) {
        let _ = (x, y);
    }
    #[zbus(property)]
    fn category(&self) -> &'static str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &'static str {
        "grajek"
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.0.items()[0].label.clone()
    }
    #[zbus(property)]
    fn status(&self) -> &'static str {
        "Active"
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> &'static str {
        ""
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        self.1.clone()
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'_> {
        ObjectPath::from_static_str_unchecked("/Menu")
    }
    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[expect(
    clippy::unused_self,
    reason = "D-Bus menu requires receivers for protocol constants"
)]
#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(
        &self,
        parent: i32,
        depth: i32,
        names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        let _ = (depth, names);
        let items = self.0.items();
        let children = if parent == 0 {
            items
                .iter()
                .map(|item| {
                    OwnedValue::try_from(zbus::zvariant::Value::from((
                        item.id,
                        properties(item),
                        Vec::<OwnedValue>::new(),
                    )))
                    .map_err(|_| fdo::Error::Failed("Could not encode tray menu".into()))
                })
                .collect::<fdo::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let mut props = items
            .iter()
            .find(|item| item.id == parent)
            .map_or_else(Properties::new, properties);
        if parent == 0 {
            props.insert(
                "children-display".into(),
                OwnedValue::from(Str::from("submenu")),
            );
        }
        Ok((self.1.load(Ordering::Acquire), (parent, props, children)))
    }
    fn get_group_properties(&self, ids: Vec<i32>, names: Vec<String>) -> Vec<(i32, Properties)> {
        drop(names);
        let result = self
            .0
            .items()
            .iter()
            .filter(|item| ids.is_empty() || ids.contains(&item.id))
            .map(|item| (item.id, properties(item)))
            .collect();
        drop(ids);
        result
    }
    fn get_property(&self, id: i32, name: &str) -> fdo::Result<OwnedValue> {
        self.0
            .items()
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| properties(item).remove(name))
            .ok_or_else(|| fdo::Error::InvalidArgs("Unknown menu item or property".into()))
    }
    fn event(&self, id: i32, event: &str, data: OwnedValue, timestamp: u32) {
        let _ = (data, timestamp);
        if event == "clicked" {
            self.0.activate(id);
        }
    }
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event, data, timestamp) in events {
            self.event(id, &event, data, timestamp);
        }
        Vec::new()
    }
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (ids, Vec::new())
    }
    fn about_to_show(&self, id: i32) -> bool {
        id == 0
    }
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &'static str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &'static str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

pub async fn run(endpoint: Endpoint, mut shutdown: watch::Receiver<bool>) {
    if let Err(error) =
        async { serve(endpoint.clone(), &mut shutdown, Builder::session()?).await }.await
    {
        tracing::warn!(%error, "System tray unavailable; close still quits");
    }
    let was_available = endpoint.available.swap(false, Ordering::AcqRel);
    // Only an established tray could have hidden the window; don't remap during startup.
    if was_available && !*shutdown.borrow() {
        let _ = endpoint.window.send(super::WindowAction::Show).await;
    }
}

async fn serve(
    endpoint: Endpoint,
    shutdown: &mut watch::Receiver<bool>,
    builder: Builder<'_>,
) -> zbus::Result<()> {
    let mut states = endpoint.player.states.clone();
    let (mut previous, mut previous_icon) = {
        let state = states.borrow_and_update();
        (super::menu(&state), Visual::from(&*state))
    };
    let revision = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1));
    let connection = builder
        .serve_at(
            "/StatusNotifierItem",
            Item(endpoint.clone(), pixmaps(previous_icon)?),
        )?
        .serve_at("/Menu", Menu(endpoint.clone(), revision.clone()))?
        .build()
        .await?;
    let watcher = zbus::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .await?;
    let host: bool = watcher
        .get_property("IsStatusNotifierHostRegistered")
        .await?;
    if !host {
        return Err(zbus::Error::Failure(
            "No StatusNotifier host is running".into(),
        ));
    }
    let _: () = watcher
        .call("RegisterStatusNotifierItem", &("/StatusNotifierItem",))
        .await?;
    endpoint.available.store(true, Ordering::Release);
    let menu = connection
        .object_server()
        .interface::<_, Menu>("/Menu")
        .await?;
    let item = connection
        .object_server()
        .interface::<_, Item>("/StatusNotifierItem")
        .await?;
    let mut owners = watcher.receive_owner_changed().await?;
    let mut hosts = watcher
        .receive_property_changed::<bool>("IsStatusNotifierHostRegistered")
        .await;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = owners.next() => break,
            () = connection.closed() => break,
            host = hosts.next() => {
                if let Some(host) = host { if !host.get().await.unwrap_or(false) { break; } }
                else { break; }
            },
            changed = states.changed() => {
                if changed.is_err() { break; }
                let (current, current_icon) = {
                    let state = states.borrow_and_update();
                    (super::menu(&state), Visual::from(&*state))
                };
                if current_icon != previous_icon {
                    item.get_mut().await.1 = pixmaps(current_icon)?;
                    Item::new_icon(item.signal_emitter()).await?;
                    previous_icon = current_icon;
                }
                if current != previous {
                    let current_revision = revision.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
                    Menu::layout_updated(menu.signal_emitter(), current_revision, 0).await?;
                    Item::new_title(item.signal_emitter()).await?;
                    previous = current;
                }
            }
        }
    }
    connection.close().await
}

#[cfg(test)]
mod tests;
