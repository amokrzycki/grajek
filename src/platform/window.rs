//! Visibility changes retain the single GPUI window and all controller/media owners.
use gpui_kit::{AnyWindowHandle, Application, AsyncApp};

#[cfg(target_os = "linux")]
pub fn application() -> (
    Application,
    impl Fn(AnyWindowHandle, bool, &mut AsyncApp) -> anyhow::Result<()>,
) {
    let modes = if std::env::var_os("ZED_HEADLESS").is_some() {
        gpui_kit::WindowingModes::HEADLESS
    } else {
        gpui_kit::WindowingModes::all()
    };
    let platform = std::rc::Rc::new(gpui_linux::LinuxPlatform::new(modes));
    (
        Application::with_platform(platform.clone()),
        move |handle, visible, cx| {
            platform.set_window_visible(handle, visible)?;
            if visible {
                handle.update(cx, |_, window, _| window.activate_window())?;
            }
            Ok(())
        },
    )
}

#[cfg(target_os = "windows")]
pub fn application() -> (
    Application,
    impl Fn(AnyWindowHandle, bool, &mut AsyncApp) -> anyhow::Result<()>,
) {
    (gpui_kit::application(), |handle, visible, cx| {
        handle.update(cx, |_, window, _| {
            grajek_smtc::set_window_visible(window, visible)
        })??;
        Ok(())
    })
}
