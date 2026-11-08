use super::*;
use crate::media::{
    Tool,
    components::status::{ComponentReport, ComponentState},
};

fn text(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> String {
    let mut text = String::new();
    act(cx, window, |window, _| {
        text = window.find(id).label().unwrap_or_default().to_owned();
    });
    text
}

#[gpui_kit::test]
fn missing_components_prompt_can_be_deferred_and_reopened_by_a_new_request(
    cx: &mut TestAppContext,
) {
    let ((window, _app, _commands, snapshots), _) = open_full(cx, state(Phase::Ready));
    snapshots.send_replace(State {
        components_request: 1,
        ..state(Phase::Ready)
    });
    cx.run_until_parked();
    assert!(present(cx, window, "components-dialog"));
    click(cx, window, "components-later");
    assert!(!present(cx, window, "components-dialog"));
    snapshots.send_replace(State {
        components_request: 1,
        ..state(Phase::Ready)
    });
    cx.run_until_parked();
    assert!(!present(cx, window, "components-dialog"));
    snapshots.send_replace(State {
        components_request: 2,
        ..state(Phase::Ready)
    });
    cx.run_until_parked();
    assert!(present(cx, window, "components-dialog"));
}

fn reports() -> Vec<ComponentReport> {
    vec![
        ComponentReport {
            tool: Tool::YtDlp,
            state: ComponentState::Missing,
            version: None,
        },
        ComponentReport {
            tool: Tool::Deno,
            state: ComponentState::Ready,
            version: Some("deno 2.9.8".into()),
        },
    ]
}

#[gpui_kit::test]
fn background_download_keeps_progress_when_closed_and_reopened(cx: &mut TestAppContext) {
    let ((window, app, mut commands, snapshots), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.open_components(cx);
        app.components.reports = reports();
        app.components.progress = crate::media::components::download::Progress::Downloading {
            bytes: 12_000_000,
            total: 66_803_068,
        };
        cx.notify();
    });
    cx.run_until_parked();
    assert!(present(cx, window, "components-cancel"));
    assert!(text(cx, window, "components-later").contains("background"));
    click(cx, window, "components-later");
    snapshots.send_replace(State {
        components_request: 1,
        ..state(Phase::Ready)
    });
    cx.run_until_parked();
    assert!(text(cx, window, "components-progress").contains("12.0 / 66.8 MB"));
    assert!(commands.try_recv().is_err());
}

#[gpui_kit::test]
fn components_repair_is_available_in_first_run_settings_without_a_token(cx: &mut TestAppContext) {
    let ((window, _app, mut commands, _), _) = open_full(cx, State::default());
    click(cx, window, "settings");
    cx.run_until_parked();
    assert!(present(cx, window, "manage-components"));
    assert!(present(cx, window, "check-components"));
    act(cx, window, |window, cx| {
        for _ in 0..16 {
            window.press("tab", cx);
            window.render_frame(cx);
            if window.find("manage-components").focused() == Some(true) {
                let action = window.find("manage-components").bounds();
                assert!(action.top() >= px(16.) && action.bottom() <= px(544.));
                window.press("enter", cx);
                return;
            }
        }
    });
    cx.run_until_parked();
    assert!(present(cx, window, "components-dialog"));
    assert!(commands.try_recv().is_err());
}

#[gpui_kit::test]
fn recovery_lists_each_tool_and_deferring_restores_focus_without_commands(cx: &mut TestAppContext) {
    let ((window, app, mut commands, _), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.open_components(cx);
        app.components.reports = reports();
    });
    cx.run_until_parked();
    assert!(present(cx, window, "component-yt-dlp"));
    assert!(present(cx, window, "component-deno"));
    assert!(text(cx, window, "component-yt-dlp").contains("Missing"));
    assert!(text(cx, window, "component-deno").contains("Ready"));
    assert!(text(cx, window, "components-download").contains("Download missing"));
    act(cx, window, |window, cx| window.press("escape", cx));
    assert!(!present(cx, window, "components-dialog"));
    act(cx, window, |window, cx| {
        assert!(
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    assert!(commands.try_recv().is_err());
}

#[gpui_kit::test]
fn recovery_keeps_actions_visible_with_details_and_long_errors_at_both_sizes(
    cx: &mut TestAppContext,
) {
    for (width, height) in [(420., 480.), (520., 560.)] {
        let ((window, app, mut commands, _), _) = open_at(cx, state(Phase::Ready), width, height);
        app.update(cx, |app, cx| {
            app.settings_open = true;
            app.open_components(cx);
            app.components.reports = reports();
            app.components.progress = crate::media::components::download::Progress::Failed("The download failed. Check your internet connection, firewall and file permissions before trying again. ".repeat(8));
            cx.notify();
        });
        cx.run_until_parked();
        click(cx, window, "components-details-toggle");
        assert!(present(cx, window, "components-details"));
        let action = bounds(cx, window, "components-download");
        let later = bounds(cx, window, "components-later");
        assert!(action.top() >= px(16.) && action.bottom() <= px(height - 16.));
        assert!(later.left() >= px(16.) && later.right() <= px(width - 16.));
        click(cx, window, "components-later");
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
}

#[gpui_kit::test]
fn failed_download_can_be_retried_and_success_returns_without_playing(cx: &mut TestAppContext) {
    let ((window, app, mut commands, _), _) = open_full(cx, state(Phase::Ready));
    app.update(cx, |app, cx| {
        app.open_components(cx);
        app.components.reports = reports();
        app.components.progress =
            crate::media::components::download::Progress::Failed("Offline".into());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(text(cx, window, "components-download").contains("Try again"));
    click(cx, window, "components-download");
    assert!(text(cx, window, "components-progress").contains("Restart"));
    app.update(cx, |app, cx| {
        app.components.progress = crate::media::components::download::Progress::Complete;
        app.components.reports = reports()
            .into_iter()
            .map(|mut r| {
                r.state = ComponentState::Ready;
                r
            })
            .collect();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(text(cx, window, "components-later").contains("Back to music"));
    click(cx, window, "components-later");
    assert!(commands.try_recv().is_err());
}
