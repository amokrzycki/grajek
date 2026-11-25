use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

use super::Tool;

static TOOLS: OnceLock<RwLock<Tools>> = OnceLock::new();
pub(crate) static EXECUTIONS: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

#[cfg(test)]
pub(crate) mod channel;
pub(crate) mod download;
pub(crate) mod execution;

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Artifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub unpacked_size: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Pin {
    pub version: String,
    #[serde(default)]
    pub ejs: String,
    #[cfg(target_os = "linux")]
    pub linux: Artifact,
    #[cfg(target_os = "windows")]
    pub windows: Artifact,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Pins {
    #[serde(rename = "yt-dlp")]
    pub ytdlp: Pin,
    pub deno: Pin,
    #[serde(default)]
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the update check"))]
    pub serial: u64,
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the update check"))]
    pub min_app: String,
}

pub(crate) fn pins() -> anyhow::Result<Pins> {
    Ok(serde_json::from_str(include_str!(
        "../../packaging/components.json"
    ))?)
}

impl Pins {
    pub fn versions(&self) -> Versions {
        Versions {
            ytdlp: self.ytdlp.version.clone(),
            ejs: self.ytdlp.ejs.clone(),
            deno: self.deno.version.clone(),
        }
    }
}

impl Pin {
    pub fn artifact(&self) -> &Artifact {
        #[cfg(target_os = "linux")]
        {
            &self.linux
        }
        #[cfg(target_os = "windows")]
        {
            &self.windows
        }
    }
}

pub(crate) mod status;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Versions {
    #[serde(rename = "yt-dlp")]
    pub ytdlp: String,
    #[serde(rename = "yt-dlp-ejs")]
    pub ejs: String,
    pub deno: String,
}

fn version_parts(version: &str) -> Option<Vec<u64>> {
    version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}

impl Versions {
    fn valid(&self) -> bool {
        [&self.ytdlp, &self.ejs, &self.deno]
            .into_iter()
            .all(|version| version_parts(version).is_some())
    }
}

fn installed_versions(directory: &Path) -> Option<Versions> {
    serde_json::from_slice::<Versions>(&std::fs::read(directory.join("components.json")).ok()?)
        .ok()
        .filter(Versions::valid)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Need {
    Missing,
    Repair,
    Update,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MissingComponents {
    managed: bool,
}

impl fmt::Display for MissingComponents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.managed {
            f.write_str("Playback components are missing or damaged. Re-download them in Settings.")
        } else {
            f.write_str("Playback components are missing. Install yt-dlp and Deno 2.3 or newer on PATH, or download them in Settings.")
        }
    }
}
impl std::error::Error for MissingComponents {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    System,
    Managed,
}

#[derive(Clone, Debug)]
pub(crate) struct Tools {
    pub mode: Mode,
    directory: Option<PathBuf>,
    pub versions: Option<Versions>,
}

impl Tools {
    fn select_with_fallback(
        force_system: bool,
        directory: Option<PathBuf>,
        fallback: Option<PathBuf>,
    ) -> Self {
        let primary = Self::select(force_system, directory);
        if !force_system && primary.mode == Mode::System {
            Self::select(false, fallback)
        } else {
            primary
        }
    }
    fn select(force_system: bool, directory: Option<PathBuf>) -> Self {
        let directory = directory.filter(|dir| {
            !force_system && dir.join("components.json").try_exists().unwrap_or(true)
        });
        let versions = directory.as_deref().and_then(installed_versions);
        Self {
            mode: if directory.is_some() {
                Mode::Managed
            } else {
                Mode::System
            },
            directory,
            versions,
        }
    }

    pub(super) fn path(&self, tool: Tool) -> PathBuf {
        self.directory.as_ref().map_or_else(
            || PathBuf::from(tool.name()),
            |dir| dir.join(tool.name()).join(tool.filename()),
        )
    }

    pub(crate) fn command(&self, tool: Tool) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(self.path(tool));
        command.stdin(std::process::Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    }

    pub(super) fn extraction_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-playlist",
            "--js-runtimes",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        if self.mode == Mode::Managed {
            let mut runtime = OsString::from("deno:");
            runtime.push(self.path(Tool::Deno));
            args.push(runtime);
        } else {
            args.extend(["deno", "--remote-components", "ejs:npm"].map(Into::into));
        }
        args.extend(crate::ytdlp::NETWORK_ARGS.map(Into::into));
        args
    }

    pub(crate) fn failure(&self) -> MissingComponents {
        MissingComponents {
            managed: self.mode == Mode::Managed,
        }
    }

    pub(super) fn valid_version(&self, tool: Tool, output: &str) -> bool {
        if self.mode == Mode::System {
            return match tool {
                Tool::YtDlp => !output.trim().is_empty(),
                Tool::Deno => super::supported_deno(output),
            };
        }
        self.versions.as_ref().is_some_and(|versions| {
            let expected = match tool {
                Tool::YtDlp => &versions.ytdlp,
                Tool::Deno => &versions.deno,
            };
            let actual = match tool {
                Tool::YtDlp => output.split_whitespace().next(),
                Tool::Deno => output
                    .strip_prefix("deno ")
                    .and_then(|s| s.split_whitespace().next()),
            };
            !expected.is_empty()
                && actual == Some(expected)
                && (tool != Tool::Deno || super::supported_deno(output))
        })
    }

    pub(crate) fn need(&self) -> Option<Need> {
        self.need_on(std::env::var_os("PATH").as_deref())
    }

    fn need_on(&self, path: Option<&OsStr>) -> Option<Need> {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt as _;
        let executable = |path: &std::path::Path| {
            std::fs::metadata(path).is_ok_and(|meta| {
                #[cfg(unix)]
                {
                    meta.is_file() && meta.permissions().mode() & 0o111 != 0
                }
                #[cfg(windows)]
                {
                    meta.is_file()
                }
            })
        };
        if self.mode == Mode::System {
            let available = path.is_some_and(|path| {
                [Tool::YtDlp, Tool::Deno].into_iter().all(|tool| {
                    std::env::split_paths(path).any(|dir| executable(&dir.join(tool.system_name())))
                })
            });
            return (!available).then_some(Need::Missing);
        }
        let Some(versions) = &self.versions else {
            return Some(Need::Repair);
        };
        if [Tool::YtDlp, Tool::Deno]
            .into_iter()
            .any(|tool| !executable(&self.path(tool)))
        {
            return Some(Need::Repair);
        }
        let expected = pins().ok()?.versions();
        [
            (&versions.ytdlp, &expected.ytdlp),
            (&versions.ejs, &expected.ejs),
            (&versions.deno, &expected.deno),
        ]
        .into_iter()
        .any(|(installed, pinned)| version_parts(installed) < version_parts(pinned))
        .then_some(Need::Update)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn initialize(directory: Option<PathBuf>) {
    initialize_with_fallback(directory, None);
}

pub(crate) fn system_override() -> bool {
    std::env::var_os("GRAJEK_TOOLS").as_deref() == Some(OsStr::new("system"))
}

pub(crate) fn initialize_with_fallback(directory: Option<PathBuf>, fallback: Option<PathBuf>) {
    let tools = Tools::select_with_fallback(system_override(), directory, fallback);
    if let Some(v) = &tools.versions {
        tracing::info!(mode = ?tools.mode, yt_dlp = %v.ytdlp, ejs = %v.ejs, deno = %v.deno, "Playback components selected");
    } else {
        tracing::info!(mode = ?tools.mode, "Playback components selected");
    }
    if TOOLS.set(RwLock::new(tools)).is_err() {
        tracing::debug!("Playback components were already initialized");
    }
}

pub(crate) fn tools() -> Tools {
    TOOLS
        .get_or_init(|| RwLock::new(Tools::select(true, None)))
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn refresh(directory: PathBuf) {
    let selection = Tools::select(system_override(), Some(directory));
    if let Some(v) = &selection.versions {
        tracing::info!(mode = ?selection.mode, yt_dlp = %v.ytdlp, ejs = %v.ejs, deno = %v.deno, "Playback components selected after download");
    }
    if let Some(lock) = TOOLS.get() {
        *lock
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = selection;
    }
}

#[cfg(test)]
mod tests;
