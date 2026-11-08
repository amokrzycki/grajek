#![cfg(target_os = "windows")]

use super::animation_fixes::focused;
use super::*;

#[gpui_kit::test]
fn missing_components_are_announced_before_the_player(cx: &mut TestAppContext) {
    let ((window, app, _commands, _snapshots), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.components_notice = Some("Playback components are missing or damaged.".into());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(present(cx, window, "components-notice"));
    click(cx, window, "components-notice-ok");
    assert!(!present(cx, window, "components-notice"));
}

#[gpui_kit::test]
fn notice_keyboard_action_restores_player_focus_without_playing(cx: &mut TestAppContext) {
    let ((window, app, mut commands, _snapshots), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.components_notice = Some("Playback components are missing or damaged.".into());
        cx.notify();
    });
    cx.run_until_parked();
    act(cx, window, TestWindowExt::render_frame);
    assert!(
        focused(cx, window, "components-notice-ok"),
        "the visible recovery action must own initial focus"
    );
    act(cx, window, |window, cx| window.press("enter", cx));
    cx.run_until_parked();
    assert!(!present(cx, window, "components-notice"));
    act(cx, window, |window, cx| {
        assert!(
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    assert!(
        commands.try_recv().is_err(),
        "continuing must not start playback or repair"
    );
}

#[gpui_kit::test]
fn notice_mouse_action_restores_settings_focus(cx: &mut TestAppContext) {
    let ((window, app, mut commands, _snapshots), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.settings_open = true;
        app.components_notice = Some("Playback components are missing or damaged.".into());
        cx.notify();
    });
    cx.run_until_parked();
    click(cx, window, "components-notice-ok");
    cx.run_until_parked();
    act(cx, window, |window, cx| {
        assert!(
            app.read(cx)
                .token
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    assert!(commands.try_recv().is_err());
}

#[gpui_kit::test]
fn recovery_content_and_action_fit_both_window_sizes(cx: &mut TestAppContext) {
    for (width, height) in [(420., 480.), (520., 560.)] {
        let ((window, app, _commands, _snapshots), _) =
            open_at(cx, state(Phase::Ready), width, height);
        app.update(cx, |app, cx| {
            app.components_notice = Some("Playback components are missing or damaged. Run Grajek Setup again to repair the installation. Additional diagnostic information must wrap without hiding the recovery action or overflowing this compact window.".into());
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            present(cx, window, "components-notice-content"),
            "recovery content must have a bounded layout"
        );
        let content = bounds(cx, window, "components-notice-content");
        let action = bounds(cx, window, "components-notice-ok");
        assert!(content.left() >= px(24.) && content.right() <= px(width - 24.));
        assert!(content.top() >= px(24.) && content.bottom() <= px(height - 24.));
        assert!(action.top() >= content.top() && action.bottom() <= content.bottom());
        assert!(action.left() >= content.left() && action.right() <= content.right());
        assert!(action.size.height >= px(32.));
    }
}
