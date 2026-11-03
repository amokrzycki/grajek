use super::*;

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
