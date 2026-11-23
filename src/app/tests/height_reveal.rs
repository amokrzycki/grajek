use super::support::configured;
use super::*;
use crate::app::motion::REVEAL;
use crate::app::motion::RevealFrame;

fn reveal(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<PlayerApp>,
) -> RevealFrame {
    drawn(cx, window);
    cx.update(|cx| view.read(cx).reveal)
}

#[gpui_kit::test]
fn opening_the_credentials_panel_reveals_and_settles_at_full_height(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, configured());
    click(cx, window, "settings");
    assert_eq!(
        reveal(cx, window, &view).credentials,
        0.0,
        "a closed panel has no height"
    );
    assert!(!present(cx, window, "bot-token"));
    click(cx, window, "change-credentials");
    assert_eq!(
        reveal(cx, window, &view).credentials,
        0.0,
        "the reveal starts at zero height"
    );
    assert!(
        present(cx, window, "bot-token"),
        "content mounts from the first frame"
    );
    assert!(
        requested(cx, window) >= 1,
        "a running reveal keeps the frame loop alive"
    );
    cx.executor().advance_clock(REVEAL / 2);
    let middle = reveal(cx, window, &view).credentials;
    assert!(
        (0.0..1.0).contains(&middle),
        "mid-reveal blends the heights: {middle}"
    );
    cx.executor().advance_clock(REVEAL);
    assert_eq!(
        reveal(cx, window, &view).credentials,
        1.0,
        "the reveal ends at full height"
    );
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(
        requested(cx, window),
        0,
        "a finished reveal requests no further frame"
    );
}

#[gpui_kit::test]
fn closing_the_credentials_panel_eases_down_and_unmounts(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, configured());
    click(cx, window, "settings");
    click(cx, window, "change-credentials");
    cx.executor().advance_clock(REVEAL * 2);
    assert_eq!(reveal(cx, window, &view).credentials, 1.0);
    click(cx, window, "change-credentials");
    assert_eq!(
        reveal(cx, window, &view).credentials,
        1.0,
        "the collapse starts from the full height"
    );
    assert!(
        present(cx, window, "bot-token"),
        "content stays mounted while collapsing"
    );
    cx.executor().advance_clock(REVEAL / 2);
    let middle = reveal(cx, window, &view).credentials;
    assert!(
        (0.0..1.0).contains(&middle),
        "the collapse eases down: {middle}"
    );
    cx.executor().advance_clock(REVEAL);
    assert_eq!(reveal(cx, window, &view).credentials, 0.0);
    assert!(
        !present(cx, window, "bot-token"),
        "collapsed content leaves the tree"
    );
}

#[gpui_kit::test]
fn reduced_motion_jumps_to_full_height_without_a_frame(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, configured());
    click(cx, window, "settings");
    cx.update(|cx| cx.set_reduce_motion(true));
    click(cx, window, "change-credentials");
    assert_eq!(
        reveal(cx, window, &view).credentials,
        1.0,
        "reduced motion reaches full height immediately"
    );
    assert!(present(cx, window, "bot-token"));
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(requested(cx, window), 0, "reduced motion requests no frame");
}

#[gpui_kit::test]
fn a_settings_round_trip_does_not_replay_an_open_panel(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, configured());
    click(cx, window, "settings");
    click(cx, window, "change-credentials");
    cx.executor().advance_clock(REVEAL * 2);
    assert_eq!(
        reveal(cx, window, &view).credentials,
        1.0,
        "the panel finished opening"
    );
    for open in [false, true] {
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.settings_open = open;
                cx.notify();
            });
        });
        drawn(cx, window);
    }
    assert_eq!(
        reveal(cx, window, &view).credentials,
        1.0,
        "still full height after the round-trip"
    );
    cx.executor().advance_clock(REVEAL * 2);
    requested(cx, window);
    drawn(cx, window);
    assert_eq!(requested(cx, window), 0, "the round-trip replays nothing");
}

#[gpui_kit::test]
fn the_preferences_disclosure_and_setup_guide_share_the_clock(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, configured());
    click(cx, window, "settings");
    click(cx, window, "preferences-disclosure");
    assert_eq!(
        reveal(cx, window, &view).preferences,
        0.0,
        "the disclosure starts at zero height"
    );
    assert!(present(cx, window, "close-to-tray"));
    cx.executor().advance_clock(REVEAL);
    assert_eq!(
        reveal(cx, window, &view).preferences,
        1.0,
        "the disclosure reaches full height"
    );
    click(cx, window, "change-credentials");
    cx.executor().advance_clock(REVEAL);
    reveal(cx, window, &view);
    click(cx, window, "setup-help");
    assert_eq!(
        reveal(cx, window, &view).guide,
        0.0,
        "the guide starts at zero height"
    );
    assert!(
        present(cx, window, "developer-portal"),
        "the guide mounts from the first frame"
    );
    cx.executor().advance_clock(REVEAL);
    assert_eq!(reveal(cx, window, &view).guide, 1.0);
}
