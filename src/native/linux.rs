//! MPRIS server on the user session bus. No polling, reconnect loop, or playback ownership.
use super::{Command, Endpoint, Snapshot, Status};
use std::{collections::HashMap, time::Duration};
use tokio::sync::watch;
use zbus::{
    Connection,
    connection::Builder,
    fdo,
    object_server::SignalEmitter,
    zvariant::{ObjectPath, OwnedValue, Str},
};

const PATH: &str = "/org/mpris/MediaPlayer2";
const NAME: &str = "org.mpris.MediaPlayer2.grajek";

struct Root;

#[expect(
    clippy::unused_self,
    reason = "D-Bus dispatch requires receivers for constant properties and unsupported methods"
)]
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("Raise is unavailable".into()))
    }
    fn quit(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "Close the Grajek window to quit".into(),
        ))
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &'static str {
        "Grajek"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<&str> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<&str> {
        vec![]
    }
}

struct Player(Endpoint);

impl Player {
    fn send(&self, command: Command) -> fdo::Result<()> {
        self.0
            .send(command)
            .map_err(|error| fdo::Error::Failed(error.into()))
    }
}

#[expect(
    clippy::unused_self,
    reason = "D-Bus dispatch requires receivers for constant properties and unsupported methods"
)]
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play(&self) -> fdo::Result<()> {
        self.send(Command::Play)
    }
    fn pause(&self) -> fdo::Result<()> {
        self.send(Command::Pause)
    }
    fn play_pause(&self) -> fdo::Result<()> {
        self.send(Command::PlayPause)
    }
    fn next(&self) -> fdo::Result<()> {
        self.send(Command::Next)
    }
    fn stop(&self) -> fdo::Result<()> {
        self.send(Command::Stop)
    }
    fn previous(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "Grajek has no Previous command".into(),
        ))
    }
    fn open_uri(&self, uri: &str) -> fdo::Result<()> {
        let _ = uri;
        Err(fdo::Error::NotSupported("Use Play URL in Grajek".into()))
    }
    fn seek(&self, offset: i64) -> fdo::Result<()> {
        if let Some(track_id) = self.0.snapshot().track_id {
            self.send(Command::SeekRelative { track_id, offset })?;
        }
        Ok(())
    }
    #[expect(
        clippy::needless_pass_by_value,
        reason = "zbus deserializes the method argument by value"
    )]
    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) -> fdo::Result<()> {
        let snapshot = self.0.snapshot();
        if position >= 0
            && let Some(id) = snapshot.track_id
            && track_path(id) == track_id
        {
            self.send(Command::Seek {
                track_id: id,
                position: Duration::from_micros(position.unsigned_abs()),
            })?;
        }
        Ok(())
    }
    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        status(self.0.snapshot().status)
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&self.0.snapshot())
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        micros(self.0.snapshot().position)
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        let s = self.0.snapshot();
        s.can_play || s.can_pause
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        let s = self.0.snapshot();
        s.can_pause || s.status == Status::Paused && s.can_play
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.0.snapshot().can_seek
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.0.snapshot().can_next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        false
    }
    // MPRIS requires this support flag to stay constant; per-command flags reflect readiness.
    #[zbus(property(emits_changed_signal = "false"))]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, value: f64) -> fdo::Result<()> {
        if value.to_bits() == 1.0_f64.to_bits() {
            Ok(())
        } else {
            Err(fdo::Error::NotSupported("Only rate 1 is supported".into()))
        }
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    // This is the bot's applied output gain, never the computer's master volume.
    #[zbus(property)]
    fn volume(&self) -> f64 {
        f64::from(self.0.snapshot().volume) / 100.0
    }
    #[zbus(property)]
    fn set_volume(&self, value: f64) -> fdo::Result<()> {
        let _ = value;
        Err(fdo::Error::NotSupported(
            "Adjust Discord bot volume in Grajek".into(),
        ))
    }
}

fn status(status: Status) -> &'static str {
    match status {
        Status::Stopped => "Stopped",
        Status::Playing => "Playing",
        Status::Paused => "Paused",
    }
}

fn micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

fn track_path(id: u64) -> ObjectPath<'static> {
    // Decimal digits appended to a fixed absolute path are valid D-Bus object paths.
    ObjectPath::try_from(format!("/io/github/amokrzycki/grajek/track/t{id}")).unwrap_or(
        ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/TrackList/NoTrack"),
    )
}

fn metadata(snapshot: &Snapshot) -> HashMap<String, OwnedValue> {
    let Some(id) = snapshot.track_id else {
        return HashMap::new();
    };
    let mut values = HashMap::from([("mpris:trackid".into(), OwnedValue::from(track_path(id)))]);
    if let Some(title) = &snapshot.title {
        values.insert(
            "xesam:title".into(),
            OwnedValue::from(Str::from(title.clone())),
        );
    }
    if let Some(duration) = snapshot.duration {
        values.insert("mpris:length".into(), OwnedValue::from(micros(duration)));
    }
    values
}

/// Keeps a well-known name exclusive: a second instance must never replace the first.
async fn connect(endpoint: Endpoint, builder: Builder<'_>, name: &str) -> zbus::Result<Connection> {
    let connection = builder
        .serve_at(PATH, Root)?
        .serve_at(PATH, Player(endpoint))?
        .build()
        .await?;
    if connection
        .request_name_with_flags(name, fdo::RequestNameFlags::DoNotQueue.into())
        .await?
        == fdo::RequestNameReply::PrimaryOwner
    {
        Ok(connection)
    } else {
        connection.close().await?;
        Err(zbus::Error::NameTaken)
    }
}

async fn publish(connection: &Connection, before: &Snapshot, after: &Snapshot) -> zbus::Result<()> {
    let interface = connection
        .object_server()
        .interface::<_, Player>(PATH)
        .await?;
    let player = interface.get().await;
    let emitter = interface.signal_emitter();
    if before.status != after.status {
        player.playback_status_changed(emitter).await?;
    }
    if before.track_id != after.track_id
        || before.title != after.title
        || before.duration != after.duration
    {
        player.metadata_changed(emitter).await?;
    }
    if before.can_play != after.can_play
        || before.can_pause != after.can_pause
        || before.status != after.status
    {
        player.can_play_changed(emitter).await?;
        player.can_pause_changed(emitter).await?;
    }
    if before.can_seek != after.can_seek {
        player.can_seek_changed(emitter).await?;
    }
    if before.can_next != after.can_next {
        player.can_go_next_changed(emitter).await?;
    }
    if before.volume != after.volume {
        player.volume_changed(emitter).await?;
    }
    drop(player);
    if before.track_id == after.track_id
        && after.track_id.is_some()
        && before.seek_serial != after.seek_serial
    {
        Player::seeked(emitter, micros(after.position)).await?;
    }
    Ok(())
}

pub async fn run(endpoint: Endpoint, mut shutdown: watch::Receiver<bool>) {
    let start = async { connect(endpoint.clone(), Builder::session()?, NAME).await };
    let connection = tokio::select! {
        biased;
        _ = shutdown.changed() => return,
        result = tokio::time::timeout(Duration::from_secs(5), start) => match result {
            Ok(Ok(connection)) => connection,
            error => { tracing::warn!(?error, "MPRIS unavailable; playback remains available"); return; }
        }
    };
    let mut states = endpoint.states.clone();
    let mut before = endpoint.snapshot();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            () = connection.closed() => { tracing::warn!("MPRIS bus disconnected; session stopped"); break; },
            result = states.changed() => {
                if result.is_err() { break; }
                let after = Snapshot::from(&*states.borrow_and_update());
                if let Err(error) = publish(&connection, &before, &after).await {
                    tracing::warn!(%error, "MPRIS publication failed; session stopped");
                    break;
                }
                before = after;
            }
        }
    }
    if let Err(error) = connection.close().await {
        tracing::warn!(%error, "MPRIS close failed");
    }
}

#[cfg(test)]
mod tests;
