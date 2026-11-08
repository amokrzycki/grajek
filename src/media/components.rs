use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

use super::Tool;

static TOOLS: OnceLock<RwLock<Tools>> = OnceLock::new();
pub(crate) static EXECUTIONS: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

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
        args.extend(
            [
                "--socket-timeout",
                "15",
                "--retries",
                "0",
                "--extractor-retries",
                "0",
            ]
            .map(Into::into),
        );
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
                    std::env::split_paths(path).any(|dir| executable(&dir.join(tool.filename())))
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

pub(crate) fn initialize_with_fallback(directory: Option<PathBuf>, fallback: Option<PathBuf>) {
    let tools = Tools::select_with_fallback(
        std::env::var_os("GRAJEK_TOOLS").as_deref() == Some(OsStr::new("system")),
        directory,
        fallback,
    );
    if let Some(v) = &tools.versions {
        tracing::info!(mode = ?tools.mode, yt_dlp = %v.ytdlp, ejs = %v.ejs, deno = %v.deno, "Playback components selected");
    } else {
        tracing::info!(mode = ?tools.mode, "Playback components selected");
    }
    let _ = TOOLS.set(RwLock::new(tools));
}

pub(crate) fn tools() -> Tools {
    TOOLS
        .get_or_init(|| RwLock::new(Tools::select(true, None)))
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn refresh(directory: PathBuf) {
    let selection = Tools::select(
        std::env::var_os("GRAJEK_TOOLS").as_deref() == Some(OsStr::new("system")),
        Some(directory),
    );
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
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[cfg(windows)]
    #[test]
    fn windows_recovery_preserves_previous_install_after_interrupted_publication() -> Result<()> {
        let dir = Fixture::new()?;
        let tools = dir.0.join("tools");
        std::fs::create_dir(&tools)?;
        std::fs::write(tools.join("components.json"), "previous marker")?;
        std::fs::rename(&tools, tools.with_extension("old"))?;
        std::fs::create_dir(tools.with_extension("partial"))?;
        super::download::recover(&tools)?;
        assert_eq!(
            std::fs::read(tools.join("components.json"))?,
            b"previous marker"
        );
        assert!(!tools.with_extension("partial").exists());
        Ok(())
    }

    #[test]
    fn user_install_precedes_bundle_but_absent_marker_preserves_bundle() -> Result<()> {
        let user = Fixture::new()?;
        let bundle = Fixture::new()?;
        std::fs::write(bundle.0.join("components.json"), "broken")?;
        let select = |force| {
            Tools::select_with_fallback(force, Some(user.0.clone()), Some(bundle.0.clone()))
        };
        assert_eq!(select(false).directory, Some(bundle.0.clone()));
        std::fs::write(user.0.join("components.json"), "broken")?;
        assert_eq!(select(false).directory, Some(user.0.clone()));
        assert_eq!(select(true).mode, Mode::System);
        Ok(())
    }

    #[tokio::test]
    async fn reports_distinguish_missing_from_present_but_broken_tools() -> Result<()> {
        use super::status::ComponentState;
        let dir = Fixture::new()?;
        std::fs::write(dir.0.join("components.json"), "broken")?;
        let tools = Tools::select(false, Some(dir.0.clone()));
        assert!(
            tools
                .inspect()
                .iter()
                .all(|r| r.state == ComponentState::Missing)
        );
        let path = tools.path(Tool::YtDlp);
        std::fs::create_dir_all(path.parent().ok_or_else(|| anyhow::anyhow!("no parent"))?)?;
        std::fs::write(path, b"not executable")?;
        let reports = tools.check().await;
        assert!(matches!(reports[0].state, ComponentState::NeedsRepair(_)));
        assert_eq!(reports[1].state, ComponentState::Missing);
        assert_eq!(reports[0].tool, Tool::YtDlp);
        assert_eq!(reports[1].tool, Tool::Deno);
        Ok(())
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "grajek-tools-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_marker_selects_only_managed_paths_even_when_a_binary_is_missing() -> Result<()> {
        let dir = Fixture::new()?;
        std::fs::write(
            dir.0.join("components.json"),
            r#"{"yt-dlp":"2026.10.26","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
        )?;
        let tools = Tools::select(false, Some(dir.0.clone()));
        assert_eq!(tools.mode, Mode::Managed);
        #[cfg(target_os = "linux")]
        assert_eq!(
            tools.path(super::super::Tool::YtDlp),
            dir.0.join("yt-dlp/yt-dlp_linux")
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            tools.path(super::super::Tool::YtDlp),
            dir.0.join("yt-dlp/yt-dlp.exe")
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            tools.path(super::super::Tool::Deno),
            dir.0.join("deno/deno")
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            tools.path(super::super::Tool::Deno),
            dir.0.join("deno/deno.exe")
        );
        Ok(())
    }

    #[test]
    fn system_override_and_absent_marker_keep_both_tools_on_path() -> Result<()> {
        let dir = Fixture::new()?;
        for force in [false, true] {
            let tools = Tools::select(force, Some(dir.0.clone()));
            assert_eq!(tools.mode, Mode::System);
            assert_eq!(
                tools.path(super::super::Tool::YtDlp),
                PathBuf::from("yt-dlp")
            );
            assert_eq!(tools.path(super::super::Tool::Deno), PathBuf::from("deno"));
        }
        std::fs::write(dir.0.join("components.json"), "broken marker")?;
        assert_eq!(Tools::select(true, Some(dir.0.clone())).mode, Mode::System);
        assert_eq!(
            Tools::select(false, Some(dir.0.clone())).mode,
            Mode::Managed
        );
        Ok(())
    }

    #[test]
    fn extraction_uses_a_single_snapshot_and_only_system_mode_can_fetch_remote_ejs() -> Result<()> {
        let dir = Fixture::new()?;
        std::fs::write(dir.0.join("components.json"), "invalid")?;
        let managed = Tools::select(false, Some(dir.0.clone()));
        let args = managed.extraction_args();
        assert!(args.iter().any(|s| s == "--no-plugin-dirs"));
        assert!(
            !args
                .iter()
                .any(|s| s == "--remote-components" || s == "ejs:npm")
        );
        let runtime = args
            .windows(2)
            .find(|s| s[0] == "--js-runtimes")
            .map(|s| &s[1]);
        #[cfg(target_os = "linux")]
        let expected = format!("deno:{}/deno/deno", dir.0.display());
        #[cfg(target_os = "windows")]
        let expected = format!("deno:{}\\deno\\deno.exe", dir.0.display());
        assert_eq!(runtime, Some(&OsString::from(expected)));
        let system = Tools::select(true, Some(dir.0.clone()));
        let args = system.extraction_args();
        assert!(
            args.windows(2)
                .any(|s| s[0] == "--js-runtimes" && s[1] == "deno")
        );
        assert!(
            args.windows(2)
                .any(|s| s[0] == "--remote-components" && s[1] == "ejs:npm")
        );
        assert!(args.iter().any(|s| s == "--no-plugin-dirs"));
        // Publishing another selection cannot mutate a request's already chosen pair.
        assert_eq!(managed.mode, Mode::Managed);
        assert!(managed.path(Tool::YtDlp).is_absolute());
        Ok(())
    }

    #[test]
    fn managed_versions_match_the_installed_marker_and_damage_never_falls_back() -> Result<()> {
        let dir = Fixture::new()?;
        std::fs::write(
            dir.0.join("components.json"),
            r#"{"yt-dlp":"2025.01.01","yt-dlp-ejs":"0.8.0","deno":"2.3.0"}"#,
        )?;
        let tools = Tools::select(false, Some(dir.0.clone()));
        assert!(tools.valid_version(Tool::YtDlp, "2025.01.01\n"));
        assert!(tools.valid_version(Tool::Deno, "deno 2.3.0 (stable)\nv8 1.2.3"));
        assert!(!tools.valid_version(Tool::YtDlp, "2026.10.26"));
        assert!(!tools.valid_version(Tool::Deno, "deno 2.9.8 (stable)"));
        std::fs::write(dir.0.join("components.json"), "{broken")?;
        let damaged = Tools::select(false, Some(dir.0.clone()));
        assert_eq!(damaged.mode, Mode::Managed);
        assert!(!damaged.valid_version(Tool::YtDlp, "2026.10.26"));
        let error = anyhow::Error::new(damaged.failure()).context("version check");
        assert!(error.downcast_ref::<MissingComponents>().is_some());
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_marker_without_binaries_asks_for_repair_instead_of_claiming_versions() -> Result<()> {
        let dir = Fixture::new()?;
        std::fs::write(
            dir.0.join("components.json"),
            r#"{"yt-dlp":"2026.10.26","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
        )?;
        let tools = Tools::select(false, Some(dir.0.clone()));
        assert_eq!(tools.mode, Mode::Managed);
        assert_eq!(tools.need(), Some(Need::Repair));
        assert!(
            tools
                .inspect()
                .iter()
                .all(|r| r.state == status::ComponentState::Missing)
        );
        for (folder, name) in [("yt-dlp", "yt-dlp.exe"), ("deno", "deno.exe")] {
            std::fs::create_dir_all(dir.0.join(folder))?;
            std::fs::write(dir.0.join(folder).join(name), "stub")?;
        }
        assert_eq!(tools.need(), None);
        assert!(
            tools
                .inspect()
                .iter()
                .all(|r| r.state == status::ComponentState::NotChecked)
        );
        Ok(())
    }

    #[test]
    fn system_checks_reject_empty_versions_and_non_deno_output() {
        let system = Tools::select(true, None);
        assert!(!system.valid_version(Tool::YtDlp, " \n"));
        assert!(!system.valid_version(Tool::Deno, "other 2.9.8"));
        assert!(system.valid_version(Tool::YtDlp, "2026.10.26\n"));
        assert!(system.valid_version(Tool::Deno, "deno 2.9.8 (stable)"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cheap_startup_checks_find_missing_and_outdated_components_without_processes() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = Fixture::new()?;
        let system = Tools::select(true, None);
        assert_eq!(system.need_on(Some(dir.0.as_os_str())), Some(Need::Missing));
        for name in ["yt-dlp", "deno"] {
            std::fs::write(dir.0.join(name), "not a runnable program")?;
            std::fs::set_permissions(dir.0.join(name), std::fs::Permissions::from_mode(0o755))?;
        }
        assert_eq!(system.need_on(Some(dir.0.as_os_str())), None);
        std::fs::write(
            dir.0.join("components.json"),
            r#"{"yt-dlp":"2025.01.01","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
        )?;
        let managed = Tools::select(false, Some(dir.0.clone()));
        assert_eq!(managed.need_on(None), Some(Need::Repair));
        for (folder, name) in [("yt-dlp", "yt-dlp_linux"), ("deno", "deno")] {
            std::fs::remove_file(dir.0.join(folder))?;
            std::fs::create_dir(dir.0.join(folder))?;
            std::fs::write(dir.0.join(folder).join(name), "not a runnable program")?;
            std::fs::set_permissions(
                dir.0.join(folder).join(name),
                std::fs::Permissions::from_mode(0o755),
            )?;
        }
        assert_eq!(managed.need_on(None), Some(Need::Update));
        Ok(())
    }
}
