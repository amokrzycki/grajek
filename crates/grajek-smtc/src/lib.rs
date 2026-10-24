//! Safe, window-scoped SMTC ownership. The application keeps its unsafe-code prohibition.
#![cfg(target_os = "windows")]
use raw_window_handle::HasWindowHandle;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub use windows::core::Result;
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
        SystemMediaTransportControlsButton, SystemMediaTransportControlsTimelineProperties,
    },
};

mod interop;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Play,
    Pause,
    Next,
    Stop,
    Seek { track_id: u64, position: Duration },
}

pub struct View<'a> {
    pub track_id: Option<u64>,
    pub title: Option<&'a str>,
    pub status: MediaStatus,
    pub duration: Option<Duration>,
    pub position: Duration,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_next: bool,
    pub can_stop: bool,
    pub can_seek: bool,
}

#[derive(Clone, Copy)]
pub enum MediaStatus {
    Stopped,
    Playing,
    Paused,
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

/// Lives on the GPUI apartment. Callbacks hold only a bounded command bridge, never UI.
pub struct Session {
    controls: SystemMediaTransportControls,
    button: Option<i64>,
    position: Option<i64>,
    alive: Arc<AtomicBool>,
    position_track: Option<u64>,
    callback: Callback,
    metadata: Option<(Option<u64>, Option<String>)>,
    // Rc makes the COM apartment guard and its owner !Send/!Sync.
    _apartment: interop::Apartment,
}

impl Session {
    /// The handle is borrowed only while GetForWindow validates/obtains the COM object.
    /// No HWND is retained or dereferenced by callbacks or by teardown.
    pub fn new(
        window: &impl HasWindowHandle,
        callback: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self> {
        let apartment = interop::Apartment::new()?;
        let controls = interop::for_window(window)?;
        let callback: Callback = Arc::new(callback);
        let mut session = Self {
            controls,
            button: None,
            position: None,
            alive: Arc::new(AtomicBool::new(true)),
            position_track: None,
            callback: callback.clone(),
            metadata: None,
            _apartment: apartment,
        };
        let button_callback = callback.clone();
        let alive = session.alive.clone();
        session.button = Some(session.controls.ButtonPressed(&TypedEventHandler::new(
            move |_,
                  args: windows::core::Ref<
                '_,
                windows::Media::SystemMediaTransportControlsButtonPressedEventArgs,
            >| {
                if alive.load(Ordering::Acquire)
                    && let Some(args) = args.as_ref()
                    && let Some(event) = button_event(args.Button()?)
                {
                    button_callback(event);
                }
                Ok(())
            },
        ))?);
        session.controls.SetIsPreviousEnabled(false)?;
        session.controls.SetIsFastForwardEnabled(false)?;
        session.controls.SetIsRewindEnabled(false)?;
        session.controls.SetIsRecordEnabled(false)?;
        session.controls.SetIsChannelUpEnabled(false)?;
        session.controls.SetIsChannelDownEnabled(false)?;
        Ok(session)
    }

    fn position_handler(&mut self, track_id: Option<u64>) -> Result<()> {
        if self.position_track == track_id {
            return Ok(());
        }
        if let Some(token) = self.position {
            self.controls.RemovePlaybackPositionChangeRequested(token)?;
            self.position = None;
        }
        self.position_track = None;
        if let Some(track_id) = track_id {
            let alive = self.alive.clone();
            let callback = self.callback.clone();
            self.position = Some(self.controls.PlaybackPositionChangeRequested(
                &TypedEventHandler::new(
                    move |_,
                          args: windows::core::Ref<
                        '_,
                        windows::Media::PlaybackPositionChangeRequestedEventArgs,
                    >| {
                        if alive.load(Ordering::Acquire)
                            && let Some(args) = args.as_ref()
                            && let Some(position) = duration(args.RequestedPlaybackPosition()?)
                        {
                            // Capture the registration's ID: a queued old delegate cannot seek its successor.
                            callback(Event::Seek { track_id, position });
                        }
                        Ok(())
                    },
                ),
            )?);
            self.position_track = Some(track_id);
        }
        Ok(())
    }

    pub fn publish(&mut self, view: &View<'_>) -> Result<()> {
        self.controls.SetIsPlayEnabled(view.can_play)?;
        self.controls.SetIsPauseEnabled(view.can_pause)?;
        self.controls.SetIsNextEnabled(view.can_next)?;
        self.controls.SetIsStopEnabled(view.can_stop)?;
        self.controls.SetPlaybackStatus(match view.status {
            MediaStatus::Stopped => MediaPlaybackStatus::Stopped,
            MediaStatus::Playing => MediaPlaybackStatus::Playing,
            MediaStatus::Paused => MediaPlaybackStatus::Paused,
        })?;
        let metadata = (view.track_id, view.title.map(str::to_owned));
        if self.metadata.as_ref() != Some(&metadata) {
            let updater = self.controls.DisplayUpdater()?;
            updater.ClearAll()?;
            updater.SetType(MediaPlaybackType::Music)?;
            updater
                .MusicProperties()?
                .SetTitle(&view.title.unwrap_or_default().into())?;
            updater.Update()?;
            self.metadata = Some(metadata);
        }
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        timeline.SetStartTime(timespan(Duration::ZERO))?;
        // An unknown duration has no honest finite timeline; clear any predecessor's range.
        let (position, end) = timeline_range(view.duration, view.position);
        timeline.SetEndTime(timespan(end))?;
        timeline.SetPosition(timespan(position))?;
        timeline.SetMinSeekTime(timespan(if view.can_seek {
            Duration::ZERO
        } else {
            position
        }))?;
        timeline.SetMaxSeekTime(timespan(if view.can_seek { end } else { position }))?;
        self.controls.UpdateTimelineProperties(&timeline)?;
        self.position_handler(view.track_id.filter(|_| view.can_seek))?;
        self.controls.SetIsEnabled(
            view.track_id.is_some() || view.can_play || view.can_next || view.can_stop,
        )?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Some(token) = self.button.take() {
            let _ = self.controls.RemoveButtonPressed(token);
        }
        if let Some(token) = self.position.take() {
            let _ = self.controls.RemovePlaybackPositionChangeRequested(token);
        }
        let _ = self.controls.SetIsEnabled(false);
        if let Ok(updater) = self.controls.DisplayUpdater() {
            let _ = updater.ClearAll();
            let _ = updater.Update();
        }
    }
}

fn button_event(button: SystemMediaTransportControlsButton) -> Option<Event> {
    match button {
        SystemMediaTransportControlsButton::Play => Some(Event::Play),
        SystemMediaTransportControlsButton::Pause => Some(Event::Pause),
        SystemMediaTransportControlsButton::Next => Some(Event::Next),
        SystemMediaTransportControlsButton::Stop => Some(Event::Stop),
        _ => None,
    }
}

fn timeline_range(duration: Option<Duration>, position: Duration) -> (Duration, Duration) {
    duration.map_or((Duration::ZERO, Duration::ZERO), |duration| {
        (position, duration.max(position))
    })
}

fn timespan(duration: Duration) -> TimeSpan {
    TimeSpan {
        Duration: i64::try_from(duration.as_nanos() / 100).unwrap_or(i64::MAX),
    }
}
fn duration(span: TimeSpan) -> Option<Duration> {
    u64::try_from(span.Duration).ok().map(|ticks| {
        Duration::from_secs(ticks / 10_000_000) + Duration::from_nanos((ticks % 10_000_000) * 100)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_events_and_timeline_units_are_checked() {
        for (button, event) in [
            (SystemMediaTransportControlsButton::Play, Event::Play),
            (SystemMediaTransportControlsButton::Pause, Event::Pause),
            (SystemMediaTransportControlsButton::Next, Event::Next),
            (SystemMediaTransportControlsButton::Stop, Event::Stop),
        ] {
            assert_eq!(button_event(button), Some(event));
        }
        assert_eq!(
            button_event(SystemMediaTransportControlsButton::Previous),
            None
        );
        assert_eq!(duration(TimeSpan { Duration: -1 }), None);
        let position = Duration::from_millis(12345);
        assert_eq!(duration(timespan(position)), Some(position));
        assert_eq!(timespan(Duration::MAX).Duration, i64::MAX);
        assert_eq!(
            timeline_range(None, position),
            (Duration::ZERO, Duration::ZERO)
        );
        assert_eq!(
            timeline_range(Some(Duration::from_secs(60)), position),
            (position, Duration::from_secs(60))
        );
    }
}
