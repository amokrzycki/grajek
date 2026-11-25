use super::*;
use crate::test_support::Scratch;
use anyhow::Result;

#[cfg(windows)]
#[test]
fn windows_recovery_preserves_previous_install_after_interrupted_publication() -> Result<()> {
    let dir = Scratch::new()?;
    let tools = dir.path().join("tools");
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
    let user = Scratch::new()?;
    let bundle = Scratch::new()?;
    std::fs::write(bundle.path().join("components.json"), "broken")?;
    let select = |force| {
        Tools::select_with_fallback(
            force,
            Some(user.path().to_path_buf()),
            Some(bundle.path().to_path_buf()),
        )
    };
    assert_eq!(select(false).directory, Some(bundle.path().to_path_buf()));
    std::fs::write(user.path().join("components.json"), "broken")?;
    assert_eq!(select(false).directory, Some(user.path().to_path_buf()));
    assert_eq!(select(true).mode, Mode::System);
    Ok(())
}

#[tokio::test]
async fn reports_distinguish_missing_from_present_but_broken_tools() -> Result<()> {
    use super::status::ComponentState;
    let dir = Scratch::new()?;
    std::fs::write(dir.path().join("components.json"), "broken")?;
    let tools = Tools::select(false, Some(dir.path().to_path_buf()));
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

#[test]
fn a_marker_selects_only_managed_paths_even_when_a_binary_is_missing() -> Result<()> {
    let dir = Scratch::new()?;
    std::fs::write(
        dir.path().join("components.json"),
        r#"{"yt-dlp":"2026.10.26","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
    )?;
    let tools = Tools::select(false, Some(dir.path().to_path_buf()));
    assert_eq!(tools.mode, Mode::Managed);
    #[cfg(target_os = "linux")]
    assert_eq!(
        tools.path(super::super::Tool::YtDlp),
        dir.path().join("yt-dlp/yt-dlp_linux")
    );
    #[cfg(target_os = "windows")]
    assert_eq!(
        tools.path(super::super::Tool::YtDlp),
        dir.path().join("yt-dlp/yt-dlp.exe")
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        tools.path(super::super::Tool::Deno),
        dir.path().join("deno/deno")
    );
    #[cfg(target_os = "windows")]
    assert_eq!(
        tools.path(super::super::Tool::Deno),
        dir.path().join("deno/deno.exe")
    );
    Ok(())
}

#[test]
fn system_override_and_absent_marker_keep_both_tools_on_path() -> Result<()> {
    let dir = Scratch::new()?;
    for force in [false, true] {
        let tools = Tools::select(force, Some(dir.path().to_path_buf()));
        assert_eq!(tools.mode, Mode::System);
        assert_eq!(
            tools.path(super::super::Tool::YtDlp),
            PathBuf::from("yt-dlp")
        );
        assert_eq!(tools.path(super::super::Tool::Deno), PathBuf::from("deno"));
    }
    std::fs::write(dir.path().join("components.json"), "broken marker")?;
    assert_eq!(
        Tools::select(true, Some(dir.path().to_path_buf())).mode,
        Mode::System
    );
    assert_eq!(
        Tools::select(false, Some(dir.path().to_path_buf())).mode,
        Mode::Managed
    );
    Ok(())
}

#[test]
fn extraction_uses_a_single_snapshot_and_only_system_mode_can_fetch_remote_ejs() -> Result<()> {
    let dir = Scratch::new()?;
    std::fs::write(dir.path().join("components.json"), "invalid")?;
    let managed = Tools::select(false, Some(dir.path().to_path_buf()));
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
    let expected = format!("deno:{}/deno/deno", dir.path().display());
    #[cfg(target_os = "windows")]
    let expected = format!("deno:{}\\deno\\deno.exe", dir.path().display());
    assert_eq!(runtime, Some(&OsString::from(expected)));
    let system = Tools::select(true, Some(dir.path().to_path_buf()));
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
    let dir = Scratch::new()?;
    std::fs::write(
        dir.path().join("components.json"),
        r#"{"yt-dlp":"2025.01.01","yt-dlp-ejs":"0.8.0","deno":"2.3.0"}"#,
    )?;
    let tools = Tools::select(false, Some(dir.path().to_path_buf()));
    assert!(tools.valid_version(Tool::YtDlp, "2025.01.01\n"));
    assert!(tools.valid_version(Tool::Deno, "deno 2.3.0 (stable)\nv8 1.2.3"));
    assert!(!tools.valid_version(Tool::YtDlp, "2026.10.26"));
    assert!(!tools.valid_version(Tool::Deno, "deno 2.9.8 (stable)"));
    std::fs::write(dir.path().join("components.json"), "{broken")?;
    let damaged = Tools::select(false, Some(dir.path().to_path_buf()));
    assert_eq!(damaged.mode, Mode::Managed);
    assert!(!damaged.valid_version(Tool::YtDlp, "2026.10.26"));
    let error = anyhow::Error::new(damaged.failure()).context("version check");
    assert!(error.downcast_ref::<MissingComponents>().is_some());
    Ok(())
}

#[cfg(target_os = "windows")]
#[test]
fn a_marker_without_binaries_asks_for_repair_instead_of_claiming_versions() -> Result<()> {
    let dir = Scratch::new()?;
    std::fs::write(
        dir.path().join("components.json"),
        r#"{"yt-dlp":"2026.10.26","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
    )?;
    let tools = Tools::select(false, Some(dir.path().to_path_buf()));
    assert_eq!(tools.mode, Mode::Managed);
    assert_eq!(tools.need(), Some(Need::Repair));
    assert!(
        tools
            .inspect()
            .iter()
            .all(|r| r.state == status::ComponentState::Missing)
    );
    for (folder, name) in [("yt-dlp", "yt-dlp.exe"), ("deno", "deno.exe")] {
        std::fs::create_dir_all(dir.path().join(folder))?;
        std::fs::write(dir.path().join(folder).join(name), "stub")?;
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
    let dir = Scratch::new()?;
    let system = Tools::select(true, None);
    assert_eq!(
        system.need_on(Some(dir.path().as_os_str()), embedded_pins),
        Some(Need::Missing)
    );
    for name in ["yt-dlp", "deno"] {
        std::fs::write(dir.path().join(name), "not a runnable program")?;
        std::fs::set_permissions(
            dir.path().join(name),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    assert_eq!(
        system.need_on(Some(dir.path().as_os_str()), embedded_pins),
        None
    );
    std::fs::write(
        dir.path().join("components.json"),
        r#"{"yt-dlp":"2025.01.01","yt-dlp-ejs":"0.8.0","deno":"2.9.8"}"#,
    )?;
    let managed = Tools::select(false, Some(dir.path().to_path_buf()));
    assert_eq!(managed.need_on(None, embedded_pins), Some(Need::Repair));
    for (folder, name) in [("yt-dlp", "yt-dlp_linux"), ("deno", "deno")] {
        std::fs::remove_file(dir.path().join(folder))?;
        std::fs::create_dir(dir.path().join(folder))?;
        std::fs::write(dir.path().join(folder).join(name), "not a runnable program")?;
        std::fs::set_permissions(
            dir.path().join(folder).join(name),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    assert_eq!(managed.need_on(None, embedded_pins), Some(Need::Update));
    Ok(())
}

#[test]
fn stored_newer_manifest_reports_update_need() -> Result<()> {
    use crate::test_support::{component_keypair, component_manifest, signed_manifest};
    let root = Scratch::new()?;
    let tools_dir = root.path().join("tools");
    std::fs::create_dir(&tools_dir)?;
    let embedded = embedded_pins()?;
    std::fs::write(
        tools_dir.join("components.json"),
        serde_json::to_vec(&embedded.versions())?,
    )?;
    for tool in [Tool::YtDlp, Tool::Deno] {
        let path = tools_dir.join(tool.name()).join(tool.filename());
        std::fs::create_dir_all(path.parent().ok_or_else(|| anyhow::anyhow!("no parent"))?)?;
        std::fs::write(&path, b"stub")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let tools = Tools::select(false, Some(tools_dir));
    assert_eq!(tools.need_on(None, embedded_pins), None);
    let (signing, key) = component_keypair();
    let mut value = component_manifest()?;
    value["yt-dlp"]["version"] = "2099.1.1".into();
    let (bytes, signature) = signed_manifest(&signing, &mut value, embedded.serial + 1, "0.1.0")?;
    super::channel::store_manifest(root.path(), &bytes, &signature)?;
    assert_eq!(
        tools.need_on(None, || pins_with(Some(root.path()), &key)),
        Some(Need::Update)
    );
    Ok(())
}
