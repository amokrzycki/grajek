use super::{Command, Phase, PlayerApp, Section, State, VolumeRequest, clock, format_progress};
use crate::{media::TrackInfo, queue::QueueItem, store::HistoryEntry};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ClipboardItem, ElementId, Entity, Focusable, ScrollDelta,
    TestAppContext, WindowBounds, WindowOptions, point, px, size,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub(in crate::app) mod animation_fixes;
mod native_ui;
mod seek;

fn state(phase: Phase) -> State {
    State {
        phase,
        available: true,
        controller_ready: true,
        ..State::default()
    }
}

fn playing(phase: Phase, title: Option<&str>, seconds: u64) -> State {
    State {
        track_id: 1,
        track: Some(TrackInfo {
            title: title.map(Into::into),
            duration: Some(Duration::from_secs(225)),
            seekable: true,
        }),
        position: Duration::from_secs(seconds),
        ..state(phase)
    }
}

type Harness = (AnyWindowHandle, Entity<PlayerApp>, TestCommands, Snapshots);

// Model the controller's transport envelope while keeping assertions about its actual action.
struct TestCommands {
    receiver: mpsc::Receiver<Command>,
    acknowledged: std::rc::Rc<std::cell::Cell<u64>>,
}

impl TestCommands {
    fn try_recv(&mut self) -> Result<Command, mpsc::error::TryRecvError> {
        match self.receiver.try_recv()? {
            Command::Transport { id, command } => {
                self.acknowledged.set(id);
                Ok(*command)
            }
            command => Ok(command),
        }
    }
}

struct Snapshots {
    sender: watch::Sender<State>,
    acknowledged: std::rc::Rc<std::cell::Cell<u64>>,
}

impl Snapshots {
    fn send_replace(&self, mut state: State) {
        state.transport_ack = self.acknowledged.get();
        self.sender.send_replace(state);
    }
}

fn open_full(cx: &mut TestAppContext, initial: State) -> (Harness, watch::Receiver<VolumeRequest>) {
    open_at(cx, initial, 520., 560.)
}

#[expect(
    clippy::expect_used,
    reason = "GPUI window setup failures must fail the test with context"
)]
fn open_at(
    cx: &mut TestAppContext,
    initial: State,
    width: f32,
    height: f32,
) -> (Harness, watch::Receiver<VolumeRequest>) {
    let (commands, received) = mpsc::channel(16);
    let (volume, volume_rx) = watch::channel(VolumeRequest::default());
    let (states, updates) = watch::channel(initial);
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(width), px(height)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut app = PlayerApp::new(window, cx, commands, volume, updates);
                    app.settings_open = false;
                    window.focus(&app.input.read(cx).focus_handle(cx), cx);
                    app
                })
            },
        )
        .expect("test window should open")
    });
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("test window should render");
    let acknowledged = std::rc::Rc::new(std::cell::Cell::new(0));
    let received = TestCommands {
        receiver: received,
        acknowledged: acknowledged.clone(),
    };
    let states = Snapshots {
        sender: states,
        acknowledged,
    };
    ((window, view, received, states), volume_rx)
}

fn open(cx: &mut TestAppContext, initial: State) -> Harness {
    open_full(cx, initial).0
}

#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .expect("test window should dispatch the click");
}

#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn button(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> (String, bool) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let snapshot = window.find(id);
        (
            snapshot.label().unwrap_or_default().to_owned(),
            // GPUI Kit's Button reports no `disabled` flag; a disabled button is not focusable.
            snapshot
                .disabled()
                .unwrap_or_else(|| snapshot.focused().is_none()),
        )
    })
    .expect("test window should expose the button")
}

#[test]
fn position_is_formatted_as_minutes_and_seconds() {
    assert_eq!(clock(Duration::ZERO), "00:00");
    assert_eq!(clock(Duration::from_millis(59_999)), "00:59");
    assert_eq!(clock(Duration::from_secs(84)), "01:24");
    assert_eq!(clock(Duration::from_secs(75 * 60 + 3)), "75:03");
    assert_eq!(
        format_progress(Duration::from_secs(84), Some(Duration::from_secs(225))),
        "01:24 / 03:45"
    );
    assert_eq!(
        format_progress(Duration::from_secs(84), None),
        "01:24 (duration unavailable)"
    );
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn paste_enter_duplicate_play_and_stop(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, state(Phase::Ready));
    cx.update_window(window, |_, window, cx| {
        window.click("url", cx);
        cx.write_to_clipboard(ClipboardItem::new_string(
            "https://youtu.be/aqz-KE-bpKQ".into(),
        ));
        window.press("ctrl-v", cx);
    })
    .expect("test window should accept clipboard paste");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
        window.press("enter", cx);
        window.click("play", cx);
    })
    .expect("test window should dispatch Enter and Play");
    assert!(
        matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/aqz-KE-bpKQ")
    );
    assert!(
        received.try_recv().is_err(),
        "busy UI must reject duplicate Play"
    );
    click(cx, window, "stop");
    assert!(matches!(received.try_recv(), Ok(Command::Stop)));
    states.send_replace(state(Phase::Ready));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.click("url", cx);
        window.press("ctrl-a", cx);
        window.input("https://music.youtube.com/watch?v=BaW_jenozKc", cx);
        window.click("play", cx);
    })
    .expect("test window should accept a replacement URL and Play");
    assert!(
        matches!(received.try_recv(), Ok(Command::Play(url)) if url.contains("music.youtube.com"))
    );
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Ready));
}

#[gpui_kit::test]
fn pause_resume_button_follows_controller_state(cx: &mut TestAppContext) {
    let (window, _view, mut received, states) = open(cx, playing(Phase::Playing, Some("Song"), 84));
    assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
    click(cx, window, "pause-resume");
    assert!(matches!(received.try_recv(), Ok(Command::Pause)));
    // Rapid repeat clicks are ignored until the controller answers.
    click(cx, window, "pause-resume");
    click(cx, window, "pause-resume");
    assert!(received.try_recv().is_err());
    assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), true));
    states.send_replace(playing(Phase::Paused, Some("Song"), 85));
    cx.run_until_parked();
    assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
    click(cx, window, "pause-resume");
    assert!(matches!(received.try_recv(), Ok(Command::Resume)));
    click(cx, window, "pause-resume");
    assert!(received.try_recv().is_err());
    states.send_replace(playing(Phase::Playing, Some("Song"), 85));
    cx.run_until_parked();
    assert_eq!(button(cx, window, "pause-resume"), ("Pause".into(), false));
}

#[gpui_kit::test]
fn failed_pause_never_leaves_a_false_paused_state(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, playing(Phase::Playing, Some("Song"), 12));
    click(cx, window, "pause-resume");
    assert!(matches!(received.try_recv(), Ok(Command::Pause)));
    // The GUI did not guess Paused while waiting.
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Playing));
    states.send_replace(State {
        error: Some("Could not pause playback. The track is no longer active.".into()),
        ..state(Phase::Error)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let app = view.read(cx);
        assert_eq!(app.state.phase, Phase::Error);
        assert!(app.state.visible_track().is_none());
        assert!(app.pending.is_none());
    });
    assert!(button(cx, window, "pause-resume").1, "no track to pause");
    click(cx, window, "pause-resume");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn stop_works_while_paused(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, playing(Phase::Paused, Some("Song"), 30));
    assert!(!button(cx, window, "stop").1);
    click(cx, window, "stop");
    assert!(matches!(received.try_recv(), Ok(Command::Stop)));
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
    assert!(button(cx, window, "pause-resume").1);
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn new_url_replaces_a_paused_track(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, playing(Phase::Paused, Some("Old"), 30));
    cx.update_window(window, |_, window, cx| {
        window.click("url", cx);
        window.input("https://youtu.be/BaW_jenozKc", cx);
        window.click("play", cx);
    })
    .expect("test window should accept a URL while paused");
    assert!(
        matches!(received.try_recv(), Ok(Command::Play(url)) if url == "https://youtu.be/BaW_jenozKc")
    );
    cx.update(|cx| {
        let app = view.read(cx);
        assert_eq!(app.state.phase, Phase::Paused);
        assert_eq!(
            app.state
                .visible_track()
                .and_then(|track| track.title.as_deref()),
            Some("Old")
        );
    });
    assert!(button(cx, window, "pause-resume").1);
}

#[gpui_kit::test]
fn missing_title_duration_and_long_titles_render(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, playing(Phase::Playing, None, 5));
    let long = "Very long title ".repeat(80);
    for track in [
        TrackInfo {
            title: None,
            duration: None,
            seekable: true,
        },
        TrackInfo {
            title: Some(long.clone()),
            duration: Some(Duration::from_hours(3)),
            seekable: true,
        },
        TrackInfo {
            title: Some("ż".repeat(5000)),
            duration: None,
            seekable: true,
        },
    ] {
        states.send_replace(State {
            track: Some(track),
            error: Some(long.clone()),
            ..playing(Phase::Paused, None, 3725)
        });
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Paused));
        // Layout must survive the frame; the window never grows or panics.
        assert_eq!(button(cx, window, "pause-resume"), ("Resume".into(), false));
    }
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn unavailable_configuration_disables_actions_but_keeps_input_usable(cx: &mut TestAppContext) {
    let (commands, mut received) = mpsc::channel(4);
    let (volume, _) = watch::channel(VolumeRequest::default());
    let (_, updates) = watch::channel(State {
        phase: Phase::Error,
        error: Some("Set DISCORD_BOT_TOKEN in .env, then restart.".into()),
        available: false,
        ..State::default()
    });
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::style::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| PlayerApp::new(window, cx, commands, volume, updates))
        })
        .expect("test window should open")
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings", cx);
        window.render_frame(cx);
        window.click("url", cx);
        window.input("https://youtu.be/aqz-KE-bpKQ", cx);
        window.press("enter", cx);
        window.click("play", cx);
        window.click("pause-resume", cx);
        window.click("stop", cx);
    })
    .expect("test window should accept input and disabled actions");
    assert!(received.try_recv().is_err());
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).input.read(cx).value(),
            "https://youtu.be/aqz-KE-bpKQ"
        );
    });
}

// ---- queue, history, volume --------------------------------------------------------

fn queue_item(id: u64) -> QueueItem {
    QueueItem {
        id,
        title_unavailable: false,
        url: format!("https://www.youtube.com/watch?v={id:0>11}"),
        title: Some(format!("Queued {id}")),
    }
}

fn history_entry(index: usize) -> HistoryEntry {
    HistoryEntry {
        url: format!("https://www.youtube.com/watch?v={index:0>11}"),
        title: Some(format!("Played {index}")),
        duration_secs: Some(225),
    }
}

fn with_lists(phase: Phase, queued: u64, played: usize) -> State {
    State {
        queue: (10..10 + queued).map(queue_item).collect(),
        history: (0..played).map(history_entry).collect(),
        ..state(phase)
    }
}

fn present(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(window, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap_or(false)
}

#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn bounds(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: impl Into<ElementId>,
) -> Bounds<gpui_kit::Pixels> {
    let id = id.into();
    cx.update_window(window, move |_, window, cx| {
        window.render_frame(cx);
        window.find(id).bounds()
    })
    .expect("test window should expose the element")
}

#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn act(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    action: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(window, |_, window, cx| action(window, cx))
        .expect("test window should accept the interaction");
}

fn show(cx: &TestAppContext, view: &Entity<PlayerApp>, section: Section) {
    cx.update(|cx| {
        view.update(cx, |app, cx| app.select_section(section, cx));
    });
}

fn slider_id(cx: &TestAppContext, view: &Entity<PlayerApp>) -> (&'static str, gpui_kit::EntityId) {
    cx.update(|cx| ("slider", view.read(cx).volume.entity_id()))
}

fn shown_volume(cx: &TestAppContext, view: &Entity<PlayerApp>) -> f32 {
    cx.update(|cx| view.read(cx).volume.read(cx).value().end())
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn add_to_queue_button_and_ctrl_enter_do_not_play(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, state(Phase::Ready));
    cx.update_window(window, |_, window, cx| {
        window.click("url", cx);
        window.input("https://youtu.be/aqz-KE-bpKQ", cx);
        window.click("enqueue", cx);
        window.press("ctrl-enter", cx);
    })
    .expect("test window should accept a URL and Add to queue");
    assert!(
        matches!(received.try_recv(), Ok(Command::Enqueue(url)) if url == "https://youtu.be/aqz-KE-bpKQ")
    );
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(_))));
    assert!(received.try_recv().is_err());
    // Queueing never changes what the window believes about playback.
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Ready));
    // It stays available while another request is in flight, but not before Discord is up.
    for phase in [
        Phase::Connecting,
        Phase::Resolving,
        Phase::Playing,
        Phase::Paused,
    ] {
        states.send_replace(state(phase));
        cx.run_until_parked();
        assert!(!button(cx, window, "enqueue").1, "{phase:?}");
    }
    states.send_replace(State {
        available: false,
        ..state(Phase::Error)
    });
    cx.run_until_parked();
    assert!(button(cx, window, "enqueue").1);
    click(cx, window, "enqueue");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn next_follows_the_queue_and_ignores_rapid_clicks(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 0, 0));
    assert!(button(cx, window, "next").1, "nothing queued");
    click(cx, window, "next");
    assert!(received.try_recv().is_err());
    states.send_replace(with_lists(Phase::Ready, 2, 0));
    cx.run_until_parked();
    assert_eq!(button(cx, window, "next"), ("Play Queue".into(), false));
    click(cx, window, "next");
    click(cx, window, "next");
    assert!(matches!(received.try_recv(), Ok(Command::Next)));
    assert!(received.try_recv().is_err(), "a double click is one Next");
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Ready));
    assert!(button(cx, window, "next").1);
    // The controller's answer re-enables it for the remaining entry.
    states.send_replace(with_lists(Phase::Resolving, 1, 0));
    cx.run_until_parked();
    assert!(
        !button(cx, window, "next").1,
        "Next works during extraction too"
    );
    for phase in [Phase::Stopping, Phase::Initializing] {
        states.send_replace(with_lists(phase, 1, 0));
        cx.run_until_parked();
        assert!(button(cx, window, "next").1, "{phase:?}");
    }
}

#[gpui_kit::test]
fn tabs_switch_between_queue_and_history(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, with_lists(Phase::Ready, 2, 3));
    assert!(present(cx, window, ("queue-row", 10_u64)));
    assert!(!present(cx, window, ("history-row", 0_usize)));
    assert!(bounds(cx, window, "queue-count").size.width > px(0.));
    act(cx, window, |window, cx| {
        window.within("sections").click(1_usize, cx);
    });
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::History));
    assert!(present(cx, window, ("history-row", 0_usize)));
    assert!(!present(cx, window, ("queue-row", 10_u64)));
    assert!(present(cx, window, "history-count"));
    act(cx, window, |window, cx| {
        window.within("sections").click(0_usize, cx);
    });
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
    assert!(present(cx, window, ("queue-row", 11_u64)));
    // The tab title carries the pending count and follows controller updates.
    states.send_replace(with_lists(Phase::Ready, 0, 3));
    cx.run_until_parked();
    assert!(present(cx, window, "queue-empty"));
}

#[gpui_kit::test]
fn ctrl_digits_switch_sections_from_the_keyboard(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 1, 1));
    act(cx, window, |window, cx| window.press("ctrl-2", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::History));
    act(cx, window, |window, cx| window.press("ctrl-1", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
    // Plain digits belong to the URL field.
    act(cx, window, |window, cx| window.press("2", cx));
    cx.update(|cx| assert_eq!(view.read(cx).section, Section::Queue));
}

#[gpui_kit::test]
fn queue_rows_send_their_own_commands(cx: &mut TestAppContext) {
    let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Playing, 3, 0));
    assert!(
        button(cx, window, ("queue-up", 10_u64)).1,
        "first row cannot move up"
    );
    assert!(
        button(cx, window, ("queue-down", 12_u64)).1,
        "last row cannot move down"
    );
    assert!(!button(cx, window, ("queue-up", 11_u64)).1);
    click(cx, window, ("queue-up", 10_u64));
    assert!(received.try_recv().is_err());
    click(cx, window, ("queue-up", 11_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 11, up: true })
    ));
    click(cx, window, ("queue-down", 10_u64));
    assert!(matches!(
        received.try_recv(),
        Ok(Command::Shift { id: 10, up: false })
    ));
    click(cx, window, ("queue-remove", 12_u64));
    assert!(matches!(received.try_recv(), Ok(Command::Dequeue(12))));
    click(cx, window, "queue-clear");
    assert!(matches!(received.try_recv(), Ok(Command::ClearQueue)));
    // Row actions never touch playback, so the window keeps showing it.
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn an_empty_queue_explains_itself_and_disables_clear(cx: &mut TestAppContext) {
    let (window, _view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 0));
    assert!(present(cx, window, "queue-empty"));
    assert!(button(cx, window, "queue-clear").1);
    click(cx, window, "queue-clear");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn history_rows_play_queue_and_forget(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 0, 3));
    show(cx, &view, Section::History);
    let url = history_entry(1).url;
    click(cx, window, ("history-queue", 1_usize));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(sent)) if sent == url));
    click(cx, window, ("history-remove", 1_usize));
    assert!(matches!(received.try_recv(), Ok(Command::ForgetHistory(sent)) if sent == url));
    click(cx, window, ("history-play", 1_usize));
    assert!(matches!(received.try_recv(), Ok(Command::Play(sent)) if sent == url));
    // Play now replaces the track like Play URL: the window waits for the controller.
    cx.update(|cx| assert_eq!(view.read(cx).state.phase, Phase::Ready));
    // While a request is in flight Play now is unavailable; queueing still works.
    assert!(button(cx, window, ("history-play", 0_usize)).1);
    assert!(!button(cx, window, ("history-queue", 0_usize)).1);
    click(cx, window, ("history-play", 0_usize));
    assert!(received.try_recv().is_err());
    states.send_replace(with_lists(Phase::Playing, 0, 3));
    cx.run_until_parked();
    assert!(
        !button(cx, window, ("history-play", 0_usize)).1,
        "replaces a playing track"
    );
}

#[gpui_kit::test]
fn clearing_history_asks_first(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 3));
    show(cx, &view, Section::History);
    assert_eq!(button(cx, window, "history-clear").0, "Clear history");
    click(cx, window, "history-clear");
    assert!(received.try_recv().is_err(), "the first click only asks");
    assert_eq!(button(cx, window, "history-clear").0, "Yes, clear");
    click(cx, window, "history-clear-cancel");
    assert_eq!(button(cx, window, "history-clear").0, "Clear history");
    assert!(received.try_recv().is_err());
    // Leaving the tab also forgets the question.
    click(cx, window, "history-clear");
    show(cx, &view, Section::Queue);
    show(cx, &view, Section::History);
    assert_eq!(button(cx, window, "history-clear").0, "Clear history");
    click(cx, window, "history-clear");
    click(cx, window, "history-clear");
    assert!(matches!(received.try_recv(), Ok(Command::ClearHistory)));
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn an_empty_history_explains_itself(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, with_lists(Phase::Ready, 0, 0));
    show(cx, &view, Section::History);
    assert!(present(cx, window, "history-empty"));
    assert!(button(cx, window, "history-clear").1);
    click(cx, window, "history-clear");
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn a_recovered_history_file_is_reported(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(
        cx,
        State {
            notice: Some("Saved history was unreadable; starting empty.".into()),
            ..with_lists(Phase::Ready, 0, 0)
        },
    );
    show(cx, &view, Section::History);
    assert!(present(cx, window, "notice"));
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn long_lists_scroll_inside_the_window(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 100, 50));
    assert!(!present(cx, window, ("queue-row", 109_u64)));
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            ("queue-row", 10_u64),
            ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        );
    })
    .expect("test window should scroll");
    let after = bounds(cx, window, ("queue-row", 109_u64));
    assert!(after.origin.y < px(560.), "last queue row is visible");
    show(cx, &view, Section::History);
    assert!(!present(cx, window, ("history-row", 49_usize)));
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            ("history-row", 0_usize),
            ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        );
    })
    .expect("test window should scroll");
    assert!(bounds(cx, window, ("history-row", 49_usize)).origin.y < px(560.));
    // The controls above the lists never move.
    assert!(button(cx, window, "pause-resume").1);
}

#[gpui_kit::test]
fn long_titles_and_errors_keep_the_layout(cx: &mut TestAppContext) {
    let long = "Very long title ".repeat(60);
    let mut initial = with_lists(Phase::Error, 2, 2);
    initial.queue[0].title = Some(long.clone());
    initial.history[0].title = Some("ż".repeat(4000));
    initial.error = Some(long);
    let (window, view, _received, _states) = open(cx, initial);
    let window_width = px(520.);
    let title = bounds(cx, window, ("queue-title", 10_u64));
    assert!(title.size.width < window_width, "queue title is truncated");
    assert_eq!(
        bounds(cx, window, ("queue-row", 10_u64)).size.height,
        bounds(cx, window, ("queue-row", 11_u64)).size.height
    );
    let error = bounds(cx, window, "error");
    assert!(
        error.size.height < px(70.),
        "error is clamped to a few lines"
    );
    assert!(error.size.width <= window_width);
    for id in ["pause-resume", "next", "stop", "play", "enqueue"] {
        let button = bounds(cx, window, id);
        assert!(
            button.bottom() < px(560.) && button.right() <= window_width,
            "{id} is clipped"
        );
    }
    show(cx, &view, Section::History);
    let title = bounds(cx, window, ("history-title", 0_usize));
    assert!(
        title.size.width < window_width,
        "history title is truncated"
    );
    // Both rows keep their height whatever the title length.
    assert_eq!(
        bounds(cx, window, ("history-row", 0_usize)).size.height,
        bounds(cx, window, ("history-row", 1_usize)).size.height
    );
}

#[gpui_kit::test]
fn focus_leaves_a_control_that_turns_off(cx: &mut TestAppContext) {
    let (window, view, _received, states) = open(cx, playing(Phase::Playing, Some("Song"), 3));
    let on_stop = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, _| window.find("stop").focused())
            .ok()
            .flatten()
            .unwrap_or(false)
    };
    for _ in 0..12 {
        if on_stop(cx) {
            break;
        }
        act(cx, window, |window, cx| window.press("tab", cx));
    }
    assert!(on_stop(cx), "Tab reaches Stop");
    // Stop turns off; typing focus must not stay on the dead button.
    states.send_replace(state(Phase::Ready));
    cx.run_until_parked();
    assert!(!on_stop(cx));
    let input_focused = cx.update_window(window, |_, window, cx| {
        view.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert_eq!(input_focused.ok(), Some(true));
}

#[gpui_kit::test]
fn stopping_from_the_keyboard_returns_focus_to_the_input(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, playing(Phase::Playing, Some("Song"), 3));
    let on_stop = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, _| window.find("stop").focused())
            .ok()
            .flatten()
            .unwrap_or(false)
    };
    for _ in 0..12 {
        if on_stop(cx) {
            break;
        }
        act(cx, window, |window, cx| window.press("tab", cx));
    }
    assert!(on_stop(cx));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Stop)));
    let input_focused = cx.update_window(window, |_, window, cx| {
        view.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert_eq!(input_focused.ok(), Some(true));
}

#[gpui_kit::test]
fn the_url_input_has_focus_and_keeps_it(cx: &mut TestAppContext) {
    let (window, view, mut received, _states) = open(cx, state(Phase::Ready));
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            view.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        })
        .unwrap_or(false)
    };
    assert!(focused(cx), "the input starts focused");
    click(cx, window, "enqueue");
    assert!(received.try_recv().is_ok());
    click(cx, window, "next");
    assert!(focused(cx), "buttons do not steal typing focus");
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn the_slider_follows_mouse_drags_and_commits_on_release(cx: &mut TestAppContext) {
    let ((window, view, _received, _states), volume) = open_full(cx, state(Phase::Playing));
    let slider = slider_id(cx, &view);
    assert!(
        (shown_volume(cx, &view) - 100.0).abs() < f32::EPSILON,
        "existing users keep full volume"
    );
    let track = bounds(cx, window, slider);
    // A click anywhere on the track jumps there and is final.
    cx.update_window(window, |_, window, cx| window.click(slider, cx))
        .expect("test window should click the slider");
    let request = *volume.borrow();
    assert!((40..=60).contains(&request.percent), "{request:?}");
    assert!(request.commit && request.seq >= 1);
    // A drag emits live values, but only the newest is kept, and it commits on release.
    let y = track.center().y;
    cx.update_window(window, |_, window, cx| {
        window.drag(
            point(track.center().x, y),
            point(track.left() + track.size.width * 0.2, y),
            cx,
        );
    })
    .expect("test window should drag the slider");
    let request = *volume.borrow();
    assert!((15..=25).contains(&request.percent), "{request:?}");
    assert!(request.commit);
    assert!(
        request.seq >= 3,
        "intermediate values were produced, then coalesced"
    );
    assert!((shown_volume(cx, &view) - f32::from(request.percent)).abs() < f32::EPSILON);
}

#[gpui_kit::test]
fn the_slider_works_from_the_keyboard(cx: &mut TestAppContext) {
    let ((window, view, _received, _states), volume) = open_full(cx, state(Phase::Paused));
    let slider = slider_id(cx, &view);
    // Clicking the slider focuses it; start from a known value.
    act(cx, window, |window, cx| {
        window.click(slider, cx);
        view.update(cx, |app, cx| {
            app.volume
                .update(cx, |slider, cx| slider.set_value(50.0, window, cx));
        });
    });
    let press = |cx: &mut TestAppContext, key: &'static str| {
        act(cx, window, move |window, cx| window.press(key, cx));
        *volume.borrow()
    };
    assert_eq!(press(cx, "right").percent, 55);
    assert_eq!(press(cx, "up").percent, 60);
    assert_eq!(press(cx, "left").percent, 55);
    assert_eq!(press(cx, "down").percent, 50);
    assert_eq!(press(cx, "pageup").percent, 60);
    assert_eq!(press(cx, "pagedown").percent, 50);
    assert_eq!(press(cx, "home").percent, 0);
    assert_eq!(press(cx, "left").percent, 0, "never below zero");
    let request = press(cx, "end");
    assert_eq!(request.percent, 100);
    assert_eq!(press(cx, "right").percent, 100, "never above one hundred");
    assert!(request.commit, "key presses are final values");
    let seq = press(cx, "x").seq;
    assert_eq!(press(cx, "a").seq, seq, "other keys are ignored");
    assert!((shown_volume(cx, &view) - 100.0).abs() < f32::EPSILON);
    assert_eq!(volume.borrow().percent, 100);
}

#[gpui_kit::test]
fn the_slider_shows_what_the_controller_applied(cx: &mut TestAppContext) {
    let ((window, view, _received, states), volume) = open_full(cx, state(Phase::Playing));
    let slider = slider_id(cx, &view);
    // Unprompted changes (restored preference) move the slider.
    states.send_replace(State {
        volume: 20,
        ..state(Phase::Playing)
    });
    cx.run_until_parked();
    assert!((shown_volume(cx, &view) - 20.0).abs() < f32::EPSILON);
    act(cx, window, |window, cx| {
        window.click(slider, cx);
        window.press("left", cx);
    });
    let seq = volume.borrow().seq;
    let wanted = shown_volume(cx, &view);
    // A snapshot from before the controller saw the request must not undo it.
    states.send_replace(State {
        volume: 20,
        volume_seq: seq - 1,
        ..state(Phase::Playing)
    });
    cx.run_until_parked();
    assert!((shown_volume(cx, &view) - wanted).abs() < f32::EPSILON);
    // When it answers having refused (track gone), the slider goes back.
    states.send_replace(State {
        volume: 20,
        volume_seq: seq,
        ..state(Phase::Error)
    });
    cx.run_until_parked();
    assert!((shown_volume(cx, &view) - 20.0).abs() < f32::EPSILON);
    assert_eq!(view_text(cx, &view), "20%");
}

fn view_text(cx: &TestAppContext, view: &Entity<PlayerApp>) -> String {
    cx.update(|cx| format!("{}%", view.read(cx).volume.read(cx).value().end().round()))
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI's test macro requires a unit-returning callback; window setup failures must fail the test"
)]
fn the_slider_is_inert_without_discord(cx: &mut TestAppContext) {
    let ((window, view, _received, _states), volume) = open_full(
        cx,
        State {
            available: false,
            ..state(Phase::Error)
        },
    );
    let slider = slider_id(cx, &view);
    cx.update_window(window, |_, window, cx| {
        window.click(slider, cx);
        window.press("left", cx);
        window.press("home", cx);
    })
    .expect("test window should accept input");
    assert_eq!(volume.borrow().seq, 0);
    assert_eq!(volume.borrow().percent, 100);
}

#[gpui_kit::test]
fn compact_windows_keep_input_and_actions_usable(cx: &mut TestAppContext) {
    for (width, height) in [(420., 480.), (520., 560.)] {
        for phase in [
            Phase::Ready,
            Phase::Initializing,
            Phase::Connecting,
            Phase::Resolving,
            Phase::Playing,
            Phase::Paused,
            Phase::Stopping,
            Phase::Disconnecting,
            Phase::Error,
        ] {
            let mut initial = with_lists(phase, 3, 3);
            initial.error = (phase == Phase::Error).then(|| "Playback failed. ".repeat(40));
            let ((window, view, _, _), _) = open_at(cx, initial, width, height);
            let input = bounds(cx, window, "url");
            assert!(
                input.size.width >= px(width - 40.),
                "URL input needs room to edit a link"
            );
            for id in [
                "play",
                "enqueue",
                "import-playlist",
                "pause-resume",
                "next",
                "stop",
                "volume-percent",
                "repeat",
                "remember-queue",
                "continue-on-error",
                "queue-clear",
                "leave-voice",
                "voice-status",
            ] {
                let control = bounds(cx, window, id);
                assert!(
                    control.right() <= px(width) && control.bottom() < px(height),
                    "{phase:?}: {id} is clipped"
                );
            }
            for section in [Section::Queue, Section::History] {
                show(cx, &view, section);
                let second_row = if section == Section::Queue {
                    bounds(cx, window, ("queue-row", 11_u64))
                } else {
                    bounds(cx, window, ("history-row", 1_usize))
                };
                assert!(
                    second_row.bottom() <= px(height - 16.),
                    "{phase:?}: list needs room for two rows"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn continue_on_error_uses_the_controller_preference(cx: &mut TestAppContext) {
    let (window, view, mut received, states) = open(cx, state(Phase::Ready));
    click(cx, window, "continue-on-error");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::ContinueOnError(true))
    ));
    cx.update(|cx| assert!(!view.read(cx).state.preferences.continue_on_error));
    states.send_replace(State {
        preferences: crate::player::Preferences {
            continue_on_error: true,
            ..Default::default()
        },
        ..state(Phase::Ready)
    });
    cx.run_until_parked();
    click(cx, window, "continue-on-error");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::ContinueOnError(false))
    ));
}

#[gpui_kit::test]
fn queue_preferences_follow_snapshots_and_work_from_keyboard(cx: &mut TestAppContext) {
    use crate::player::RepeatMode;
    let (window, view, mut received, states) = open(cx, with_lists(Phase::Ready, 2, 0));
    assert_eq!(button(cx, window, "repeat").0, "Repeat: Off");
    assert_eq!(
        button(cx, window, "remember-queue").0,
        "Remember queue: Off"
    );
    for mode in [RepeatMode::Off, RepeatMode::Track, RepeatMode::Queue] {
        states.send_replace(State {
            preferences: crate::player::Preferences {
                repeat: mode,
                ..Default::default()
            },
            ..with_lists(Phase::Ready, 2, 0)
        });
        cx.run_until_parked();
        assert_eq!(button(cx, window, "repeat").0, mode.label());
        // Native Tab reaches the new control; Enter dispatches the next explicit enum mode.
        for _ in 0..20 {
            let focused = cx
                .update_window(window, |_, window, _| window.find("repeat").focused())
                .ok()
                .flatten();
            if focused == Some(true) {
                break;
            }
            act(cx, window, |window, cx| window.press("tab", cx));
        }
        act(cx, window, |window, cx| window.press("enter", cx));
        assert!(matches!(received.try_recv(), Ok(Command::Repeat(next)) if next == mode.next()));
        assert_eq!(
            button(cx, window, "repeat").0,
            mode.label(),
            "wait for controller response"
        );
    }
    click(cx, window, "remember-queue");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::RememberQueue(true))
    ));
    states.send_replace(State {
        preferences: crate::player::Preferences {
            remember_queue: true,
            ..Default::default()
        },
        ..with_lists(Phase::Ready, 2, 0)
    });
    cx.run_until_parked();
    assert_eq!(button(cx, window, "remember-queue").0, "Remember queue: On");
    click(cx, window, "remember-queue");
    assert!(matches!(
        received.try_recv(),
        Ok(Command::RememberQueue(false))
    ));
    show(cx, &view, Section::History);
    assert!(present(cx, window, "remember-queue"));
    states.send_replace(State {
        available: false,
        ..state(Phase::Error)
    });
    cx.run_until_parked();
    for id in ["repeat", "remember-queue", "next"] {
        assert!(button(cx, window, id).1);
        click(cx, window, id);
    }
    assert!(received.try_recv().is_err());
}

#[gpui_kit::test]
fn restored_lists_are_read_only_after_startup_failure_but_editable_after_gateway_loss(
    cx: &mut TestAppContext,
) {
    let restored = State {
        phase: Phase::Error,
        error: Some("Set a real DISCORD_BOT_TOKEN in .env, then restart.".into()),
        queue: vec![queue_item(10), queue_item(11)],
        history: vec![history_entry(0)],
        ..State::default()
    };
    let (window, view, mut received, states) = open(cx, restored.clone());
    for id in [
        ("queue-up", 11_u64),
        ("queue-down", 10),
        ("queue-remove", 10),
    ] {
        assert!(button(cx, window, id).1);
        click(cx, window, id);
    }
    assert!(button(cx, window, "queue-clear").1);
    click(cx, window, "queue-clear");
    show(cx, &view, Section::History);
    for id in [
        ElementId::from(("history-remove", 0_usize)),
        ElementId::from("history-clear"),
    ] {
        assert!(button(cx, window, id.clone()).1);
        click(cx, window, id);
    }
    assert!(received.try_recv().is_err());
    // An initialized controller stays able to handle local edits when Discord goes down.
    states.send_replace(State {
        controller_ready: true,
        ..restored
    });
    cx.run_until_parked();
    assert!(!button(cx, window, ("history-remove", 0_usize)).1);
    click(cx, window, ("history-remove", 0_usize));
    assert!(matches!(received.try_recv(), Ok(Command::ForgetHistory(_))));
    show(cx, &view, Section::Queue);
    assert!(!button(cx, window, "queue-clear").1);
    click(cx, window, "queue-clear");
    assert!(matches!(received.try_recv(), Ok(Command::ClearQueue)));
}

#[gpui_kit::test]
fn keyboard_can_leave_a_preference_disabled_by_gateway_loss(cx: &mut TestAppContext) {
    for id in ["repeat", "remember-queue"] {
        let (window, view, _received, states) = open(cx, state(Phase::Ready));
        for _ in 0..20 {
            if cx
                .update_window(window, |_, window, _| window.find(id).focused())
                .ok()
                .flatten()
                == Some(true)
            {
                break;
            }
            act(cx, window, |window, cx| window.press("tab", cx));
        }
        assert_eq!(
            cx.update_window(window, |_, window, _| window.find(id).focused())
                .ok()
                .flatten(),
            Some(true)
        );
        states.send_replace(State {
            available: false,
            ..state(Phase::Error)
        });
        cx.run_until_parked();
        for _ in 0..3 {
            act(cx, window, |window, cx| window.press("tab", cx));
            if cx
                .update_window(window, |_, window, cx| {
                    view.read(cx)
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                })
                .ok()
                == Some(true)
            {
                break;
            }
        }
        let focused = cx
            .update_window(window, |_, window, cx| {
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            })
            .ok();
        assert_eq!(
            focused,
            Some(true),
            "{id}: Tab cycles through local import and returns to the input"
        );
    }
}

#[gpui_kit::test]
fn leave_voice_is_native_keyboard_accessible_and_waits_for_controller(cx: &mut TestAppContext) {
    let initial = State {
        voice_connected: true,
        ..playing(Phase::Paused, Some("Żółć Привет 🎵"), 12)
    };
    let (window, view, mut received, states) = open(cx, initial.clone());
    assert_eq!(
        button(cx, window, "leave-voice"),
        ("Leave voice".into(), false)
    );
    for _ in 0..20 {
        if cx
            .update_window(window, |_, window, _| window.find("leave-voice").focused())
            .ok()
            .flatten()
            == Some(true)
        {
            break;
        }
        act(cx, window, |window, cx| window.press("tab", cx));
    }
    assert_eq!(
        cx.update_window(window, |_, window, _| window.find("leave-voice").focused())
            .ok()
            .flatten(),
        Some(true)
    );
    act(cx, window, |window, cx| window.press("space", cx));
    assert!(matches!(received.try_recv(), Ok(Command::LeaveVoice)));
    click(cx, window, "leave-voice");
    assert!(received.try_recv().is_err());
    cx.update(|cx| {
        assert!(view.read(cx).state.voice_connected);
        assert_eq!(view.read(cx).state.phase, Phase::Paused);
    });
    assert_eq!(
        cx.update_window(window, |_, window, cx| view
            .read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window))
            .ok(),
        Some(true)
    );
    for phase in [Phase::Disconnecting, Phase::Ready] {
        states.send_replace(State {
            phase,
            voice_connected: phase == Phase::Disconnecting,
            ..initial.clone()
        });
        cx.run_until_parked();
        assert!(button(cx, window, "leave-voice").1);
        click(cx, window, "leave-voice");
        assert!(received.try_recv().is_err());
    }
}

#[gpui_kit::test]
fn queue_metadata_states_and_unicode_keep_row_actions_visible(cx: &mut TestAppContext) {
    for (width, height) in [(420., 480.), (520., 560.)] {
        for title in [
            None,
            Some("Żółć Привет 🎵".repeat(30)),
            Some("x".repeat(300)),
        ] {
            for failed in [false, true] {
                let mut initial = with_lists(Phase::Playing, 3, 0);
                initial.voice_connected = true;
                initial.queue[0].title = title.clone();
                initial.queue[0].title_unavailable = failed;
                let ((window, _, _, _), _) = open_at(cx, initial, width, height);
                let row = bounds(cx, window, ("queue-row", 10_u64));
                let title = bounds(cx, window, ("queue-title", 10_u64));
                let up = bounds(cx, window, ("queue-up", 10_u64));
                let down = bounds(cx, window, ("queue-down", 10_u64));
                let remove = bounds(cx, window, ("queue-remove", 10_u64));
                assert!(
                    title.right() <= up.left()
                        && up.right() <= down.left()
                        && down.right() <= remove.left()
                );
                assert!(remove.right() <= row.right() && row.right() <= px(width));
                assert!(title.size.width > px(100.));
                assert!(button(cx, window, ("queue-up", 10_u64)).1);
                assert!(button(cx, window, ("queue-down", 12_u64)).1);
                let voice = bounds(cx, window, "voice-status");
                let leave = bounds(cx, window, "leave-voice");
                assert!(
                    voice.right() <= leave.left()
                        && leave.right() <= bounds(cx, window, "url").right(),
                    "width={width} voice={voice:?} leave={leave:?}"
                );
                assert_eq!(button(cx, window, "leave-voice").0, "Leave voice");
            }
        }
    }
}

#[gpui_kit::test]
fn playlist_import_is_explicit_and_cancellation_recovers_focus(cx: &mut TestAppContext) {
    use crate::player::ImportStatus;
    let initial = playing(Phase::Playing, Some("Current track"), 27);
    let (window, view, mut received, states) = open(cx, initial.clone());
    act(cx, window, |window, cx| {
        window.click("url", cx);
        window.input(
            "https://youtube.com/watch?v=aaaaaaaaaaa&list=PL_test123",
            cx,
        );
    });
    assert!(
        received.try_recv().is_err(),
        "pasting never starts playback"
    );
    click(cx, window, "import-playlist");
    assert!(
        matches!(received.try_recv(), Ok(Command::ImportPlaylist(url)) if url.contains("list=PL_test123"))
    );
    click(cx, window, "import-playlist");
    assert!(received.try_recv().is_err());
    assert_eq!(
        button(cx, window, "import-playlist"),
        ("Import Playlist".into(), true)
    );
    states.send_replace(State {
        import: ImportStatus::Importing,
        ..initial.clone()
    });
    cx.run_until_parked();
    assert_eq!(
        button(cx, window, "import-playlist"),
        ("Cancel Import".into(), false)
    );
    assert!(!button(cx, window, "pause-resume").1);
    assert!(
        !button(cx, window, "queue-clear").1,
        "even an empty queue can invalidate import"
    );
    for _ in 0..20 {
        if cx
            .update_window(window, |_, window, _| {
                window.find("import-playlist").focused()
            })
            .ok()
            .flatten()
            == Some(true)
        {
            break;
        }
        act(cx, window, |window, cx| window.press("tab", cx));
    }
    assert_eq!(
        cx.update_window(window, |_, window, _| window
            .find("import-playlist")
            .focused())
            .ok()
            .flatten(),
        Some(true)
    );
    act(cx, window, |window, cx| window.press("space", cx));
    assert!(matches!(received.try_recv(), Ok(Command::CancelImport)));
    states.send_replace(State {
        import: ImportStatus::Cancelled,
        ..initial
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update_window(window, |_, window, cx| view
            .read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window))
            .ok(),
        Some(true)
    );
    act(cx, window, |window, cx| window.press("ctrl-enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Enqueue(url)) if url.contains("list=")));
    act(cx, window, |window, cx| window.press("enter", cx));
    assert!(matches!(received.try_recv(), Ok(Command::Play(url)) if url.contains("list=")));
    cx.update(|cx| assert_eq!(view.read(cx).state.position, Duration::from_secs(27)));
}

#[gpui_kit::test]
fn import_feedback_and_actions_fit_both_native_sizes(cx: &mut TestAppContext) {
    use crate::player::ImportStatus;
    for (width, height) in [(420., 480.), (520., 560.)] {
        for import in [
            ImportStatus::Importing,
            ImportStatus::Cancelled,
            ImportStatus::Finished {
                added: 12,
                unavailable: 3,
                omitted: 77,
                limited: true,
            },
            ImportStatus::Failed(
                "Playlist unavailable. Check the public link and network, then try again.".into(),
            ),
        ] {
            let mut initial = with_lists(Phase::Paused, 3, 3);
            initial.error = Some("A long playback error. ".repeat(40));
            initial.import = import.clone();
            initial.preferences.repeat = crate::player::RepeatMode::Queue;
            let ((window, view, _, _), _) = open_at(cx, initial, width, height);
            let right = bounds(cx, window, "url").right();
            let mut edge = px(0.);
            for id in ["repeat", "play", "enqueue", "import-playlist"] {
                let button = bounds(cx, window, id);
                assert!(
                    button.left() >= edge && button.right() <= right,
                    "{width}: overlapping {id}, edge={edge:?}, bounds={button:?}"
                );
                edge = button.right();
            }
            let status = cx
                .update_window(window, |_, window, _| {
                    window
                        .find("import-status")
                        .label()
                        .unwrap_or_default()
                        .to_owned()
                })
                .ok();
            assert_eq!(status, Some(import.message()));
            for section in [Section::Queue, Section::History] {
                show(cx, &view, section);
                let second = if section == Section::Queue {
                    bounds(cx, window, ("queue-row", 11_u64))
                } else {
                    bounds(cx, window, ("history-row", 1_usize))
                };
                assert!(
                    second.bottom() <= px(height - 16.),
                    "import + error must leave two rows"
                );
            }
        }
    }
    let mut full = state(Phase::Ready);
    full.queue = (0..100)
        .map(|id| QueueItem {
            id,
            url: "https://www.youtube.com/watch?v=aaaaaaaaaaa".into(),
            title: Some("Track".into()),
            title_unavailable: false,
        })
        .collect();
    let (window, _, mut received, _) = open(cx, full.clone());
    assert!(button(cx, window, "import-playlist").1);
    click(cx, window, "import-playlist");
    assert!(received.try_recv().is_err());
    full.import = ImportStatus::Importing;
    let (window, _, _, _) = open(cx, full);
    assert!(
        !button(cx, window, "import-playlist").1,
        "cancel stays available at capacity"
    );
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "GPUI test setup failure must fail the test"
)]
fn first_run_token_controls_fit_minimum_size_and_clear_secret_after_submit(
    cx: &mut TestAppContext,
) {
    let ((window, view, mut received, _), _) = open_at(cx, State::default(), 420., 480.);
    click(cx, window, "settings");
    for id in ["bot-token", "save-token", "session-token", "setup-help"] {
        let control = bounds(cx, window, id);
        assert!(
            control.right() <= px(420.) && control.bottom() <= px(480.),
            "{id} must be reachable on first run"
        );
    }
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |app, cx| {
            app.token.update(cx, |input, cx| {
                input.set_value("test-bot-secret", window, cx);
            });
        });
        window.render_frame(cx);
    })
    .expect("test window should render token entry");
    click(cx, window, "session-token");
    assert!(
        matches!(received.try_recv(), Ok(Command::Setup(crate::setup::Request::Authenticate { token, remember: false })) if token == "test-bot-secret")
    );
    cx.update(|cx| assert_eq!(view.read(cx).token.read(cx).value().as_ref(), ""));
}
