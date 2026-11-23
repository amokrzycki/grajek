use super::*;
use crate::app::sections::TrackTarget;

/// Runs the pending frame callbacks until none remain, failing after a bounded
/// number of rounds so a never-settling frame loop cannot hang the test.
fn drained(cx: &mut TestAppContext, window: AnyWindowHandle) -> usize {
    for _ in 0..8 {
        let pending = requested(cx, window);
        if pending == 0 {
            return 0;
        }
        drawn(cx, window);
    }
    requested(cx, window)
}

fn open_queue_options(cx: &TestAppContext, view: &Entity<PlayerApp>, open: bool) {
    cx.update(|cx| {
        view.update(cx, |app, cx| {
            app.queue_options_open = open;
            cx.notify();
        });
    });
}

fn open_row_menu(cx: &TestAppContext, view: &Entity<PlayerApp>, row: Option<u64>) {
    cx.update(|cx| {
        view.update(cx, |app, cx| {
            app.row_menu = row.map(TrackTarget::Queue);
            cx.notify();
        });
    });
}

/// The content bounds from the first frame that mounts the menu.
fn mount_top(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    content: impl Into<ElementId> + Clone,
) -> Option<gpui_kit::Pixels> {
    let content = content.into();
    cx.update_window(window, move |_, window, cx| {
        for _ in 0..4 {
            window.render_frame(cx);
            if let Some(content) = window.try_find(content.clone()) {
                return Some(content.bounds().top());
            }
        }
        None
    })
    .ok()
    .flatten()
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "the menu must mount for the motion assertion to be meaningful"
)]
fn the_queue_options_menu_slides_down_on_open(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, state(Phase::Ready));
    open_queue_options(cx, &view, true);
    let mounting = mount_top(cx, window, "queue-options-menu").expect("the menu mounts");
    open_queue_options(cx, &view, false);
    drawn(cx, window);
    open_queue_options(cx, &view, true);
    let reopening = mount_top(cx, window, "queue-options-menu").expect("the menu mounts again");
    cx.update(|cx| cx.set_reduce_motion(true));
    let resting = bounds(cx, window, "queue-options-menu").top();
    let slide = f32::from(mounting - resting);
    assert!(
        slide < -1.0,
        "the content starts above its resting position before settling, got {slide}"
    );
    assert!(
        reopening < resting,
        "closing and reopening replays the enter"
    );
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "the menu must mount for the motion assertion to be meaningful"
)]
fn a_row_menu_slides_down_on_open(cx: &mut TestAppContext) {
    let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 2, 0));
    open_row_menu(cx, &view, Some(10));
    let mounting = mount_top(cx, window, "track-options-menu").expect("the row menu mounts");
    cx.update(|cx| cx.set_reduce_motion(true));
    let resting = bounds(cx, window, "track-options-menu").top();
    let slide = f32::from(mounting - resting);
    assert!(
        slide < -1.0,
        "the row menu starts above its resting position before settling, got {slide}"
    );
}

#[gpui_kit::test]
#[expect(
    clippy::expect_used,
    reason = "the menus must mount for the reduced-motion assertions to be meaningful"
)]
fn reduced_motion_mounts_menus_without_motion_or_animation_frames(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, view, _received, _states) = open(cx, with_lists(Phase::Ready, 2, 0));

    open_queue_options(cx, &view, true);
    let mounting = mount_top(cx, window, "queue-options-menu").expect("the menu mounts");
    assert_eq!(
        bounds(cx, window, "queue-options-menu").top(),
        mounting,
        "reduced motion renders the menu at its resting position"
    );
    assert_eq!(
        drained(cx, window),
        0,
        "an unwrapped menu starts no animation frame loop"
    );

    open_queue_options(cx, &view, false);
    open_row_menu(cx, &view, Some(10));
    let mounting = mount_top(cx, window, "track-options-menu").expect("the row menu mounts");
    assert_eq!(
        bounds(cx, window, "track-options-menu").top(),
        mounting,
        "reduced motion renders the row menu at its resting position"
    );
    assert_eq!(
        drained(cx, window),
        0,
        "an unwrapped row menu starts no animation frame loop"
    );
}
