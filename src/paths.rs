//! Per-user locations for saved state and downloaded playback components.

use std::{ffi::OsString, path::PathBuf};

/// `$XDG_DATA_HOME/grajek`, else `$HOME/.local/share/grajek`. Relative XDG values are
/// invalid per the specification and ignored.
#[cfg(any(target_os = "linux", test))]
pub fn data_dir(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("grajek"))
}

#[cfg(any(target_os = "windows", test))]
pub fn windows_data_dir(local: Option<OsString>) -> Option<PathBuf> {
    local
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Grajek"))
}

pub fn app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_data_dir(std::env::var_os("LOCALAPPDATA"))
    }
    #[cfg(target_os = "linux")]
    {
        data_dir(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
    }
}

pub fn tools_dir() -> Option<PathBuf> {
    app_data_dir().map(|dir| dir.join("tools"))
}

#[cfg(test)]
mod tests {
    use super::{data_dir, windows_data_dir};

    #[test]
    fn data_dir_follows_the_xdg_rules() {
        let root = std::env::temp_dir();
        let home = root.join("home-u");
        let xdg = root.join("xdg-data");
        let home_os = Some(home.clone().into_os_string());
        assert_eq!(
            data_dir(Some(xdg.clone().into_os_string()), home_os.clone()),
            Some(xdg.join("grajek"))
        );
        for xdg in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                data_dir(xdg, home_os.clone()),
                Some(home.join(".local/share/grajek"))
            );
        }
        assert_eq!(data_dir(None, Some("".into())), None);
        assert_eq!(data_dir(None, None), None);
        assert_eq!(
            windows_data_dir(Some(root.clone().into_os_string())),
            Some(root.join("Grajek"))
        );
        for value in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(windows_data_dir(value), None);
        }
    }
}
