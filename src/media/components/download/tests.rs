use super::*;
use sha2::Sha256;
use std::{
    io::{Cursor, Write as _},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{io::AsyncReadExt as _, net::TcpListener, task::JoinHandle};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "grajek-download-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn tools(&self) -> PathBuf {
        self.0.join("tools")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server {
    url: String,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(bytes: Vec<u8>, truncated: bool) -> Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0; 4096];
            let _ = socket.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let body = if truncated {
                &bytes[..bytes.len() / 2]
            } else {
                &bytes
            };
            let _ = socket.write_all(body).await;
        }
    });
    Ok(Server { url, task })
}

#[cfg(unix)]
fn archive() -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
    for (name, version) in [
        ("yt-dlp_linux", "2026.10.26"),
        ("deno", "deno 2.9.8 (stable)"),
    ] {
        zip.start_file(name, options)?;
        write!(
            zip,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] || exit 3\nprintf '%s\\n' '{version}'\n"
        )?;
    }
    zip.start_file("_internal/yt_dlp_ejs/yt/solver/core.min.js", options)?;
    zip.write_all(b"verified test EJS")?;
    Ok(zip.finish()?.into_inner())
}

#[cfg(windows)]
fn archive() -> Result<Vec<u8>> {
    use std::sync::OnceLock;
    static EXECUTABLE: OnceLock<std::result::Result<Vec<u8>, String>> = OnceLock::new();
    let exe = EXECUTABLE
        .get_or_init(|| {
            (|| -> Result<Vec<u8>> {
                let dir = Fixture::new()?;
                let source = dir.0.join("fixture.rs");
                std::fs::write(
                    &source,
                    r#"fn main() {
                let exe = std::env::current_exe().unwrap();
                if exe.file_name().unwrap() == "deno.exe" {
                    println!("deno 2.9.8 (stable)");
                } else { println!("2026.10.26"); }
            }"#,
                )?;
                let binary = dir.0.join("fixture.exe");
                let output = std::process::Command::new("rustc")
                    .arg(&source)
                    .arg("-o")
                    .arg(&binary)
                    .output()?;
                ensure!(
                    output.status.success(),
                    "Fixture compilation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                Ok(std::fs::read(binary)?)
            })()
            .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in ["yt-dlp.exe", "deno.exe"] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())?;
        zip.write_all(exe)?;
    }
    zip.start_file(
        "_internal/yt_dlp_ejs/yt/solver/core.min.js",
        zip::write::SimpleFileOptions::default(),
    )?;
    zip.write_all(b"verified test EJS")?;
    Ok(zip.finish()?.into_inner())
}

fn artifact_mut(pin: &mut super::super::Pin) -> &mut super::super::Artifact {
    #[cfg(unix)]
    {
        &mut pin.linux
    }
    #[cfg(windows)]
    {
        &mut pin.windows
    }
}

fn pins_for(url: &str, bytes: &[u8]) -> Result<Pins> {
    let mut pins = super::super::pins()?;
    let unpacked_size = u64::try_from(
        zip::ZipArchive::new(Cursor::new(bytes))?
            .decompressed_size()
            .unwrap_or(0),
    )?;
    for pin in [&mut pins.ytdlp, &mut pins.deno] {
        *artifact_mut(pin) = super::super::Artifact {
            url: url.into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: u64::try_from(bytes.len())?,
            unpacked_size,
        };
    }
    Ok(pins)
}

#[tokio::test]
async fn verified_archives_publish_a_complete_pair_and_marker() -> Result<()> {
    let dir = Fixture::new()?;
    let bytes = archive()?;
    let server = server(bytes.clone(), false).await?;
    let pins = pins_for(&server.url, &bytes)?;
    let (progress, rx) = watch::channel(Progress::Ready);
    let (_cancel, cancelled) = watch::channel(false);
    let _publication = tokio::time::timeout(
        Duration::from_secs(10),
        install_with(
            &dir.tools(),
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        ),
    )
    .await??;
    assert!(
        dir.tools()
            .join("yt-dlp")
            .join(Tool::YtDlp.filename())
            .is_file()
    );
    assert!(
        dir.tools()
            .join("deno")
            .join(Tool::Deno.filename())
            .is_file()
    );
    assert!(
        dir.tools()
            .join("yt-dlp/_internal/yt_dlp_ejs/yt/solver/core.min.js")
            .is_file()
    );
    let marker: super::super::Versions =
        serde_json::from_slice(&std::fs::read(dir.tools().join("components.json"))?)?;
    assert_eq!(marker.ytdlp, "2026.10.26");
    assert_eq!(marker.deno, "2.9.8");
    assert!(!dir.0.join("tools.partial").exists());
    assert!(!dir.0.join("tools.old").exists());
    assert_eq!(*rx.borrow(), Progress::Checking);
    Ok(())
}

#[tokio::test]
async fn installed_checks_reload_external_updates_and_damage_under_the_lock() -> Result<()> {
    let dir = Fixture::new()?;
    let bytes = archive()?;
    let pins = pins_for("https://example.invalid", &bytes)?;
    let zip = dir.0.join("tools.zip");
    std::fs::write(&zip, &bytes)?;
    let (_cancel, cancelled) = watch::channel(false);
    for tool in [Tool::YtDlp, Tool::Deno] {
        extract(
            &zip,
            &dir.tools().join(tool.name()),
            pins.ytdlp.artifact().unpacked_size,
            &cancelled,
        )?;
    }
    let marker = dir.tools().join("components.json");
    std::fs::write(
        &marker,
        br#"{"yt-dlp":"2025.01.01","yt-dlp-ejs":"0.8.0","deno":"2.3.0"}"#,
    )?;
    let stale = Tools::select(false, Some(dir.tools()));
    std::fs::write(&marker, serde_json::to_vec(&pins.versions())?)?;
    assert!(
        stale
            .check()
            .await
            .iter()
            .all(|report| report.state == super::super::status::ComponentState::Ready)
    );
    let cached = Tools::select(false, Some(dir.tools()));
    std::fs::write(&marker, b"{damaged")?;
    assert!(cached.check().await.iter().all(|report| matches!(
        report.state,
        super::super::status::ComponentState::NeedsRepair(_)
    )));
    Ok(())
}

fn old_install(dir: &Fixture) -> Result<()> {
    std::fs::create_dir(dir.tools())?;
    std::fs::write(dir.tools().join("components.json"), b"previous marker")?;
    std::fs::write(dir.tools().join("keep"), b"previous tools")?;
    Ok(())
}

fn assert_old_preserved(dir: &Fixture) -> Result<()> {
    assert_eq!(std::fs::read(dir.tools().join("keep"))?, b"previous tools");
    assert_eq!(
        std::fs::read(dir.tools().join("components.json"))?,
        b"previous marker"
    );
    assert!(!dir.0.join("tools.partial").exists());
    Ok(())
}

#[tokio::test]
async fn wrong_hash_and_interrupted_download_never_replace_existing_tools() -> Result<()> {
    for truncated in [false, true] {
        let dir = Fixture::new()?;
        old_install(&dir)?;
        let bytes = archive()?;
        let server = server(bytes.clone(), truncated).await?;
        let mut pins = pins_for(&server.url, &bytes)?;
        if !truncated {
            artifact_mut(&mut pins.ytdlp).sha256 = "0".repeat(64);
        }
        let (progress, _rx) = watch::channel(Progress::Ready);
        let (_cancel, cancelled) = watch::channel(false);
        let error = install_with(
            &dir.tools(),
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        )
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("unverified archive installed"))?;
        if !truncated {
            assert!(error.to_string().contains("checksum"), "{error:#}");
        }
        assert_old_preserved(&dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn traversal_and_oversized_archives_are_rejected_without_publishing() -> Result<()> {
    for name in ["../escaped", "safe"] {
        let dir = Fixture::new()?;
        old_install(&dir)?;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(name, zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"untrusted")?;
        let bytes = zip.finish()?.into_inner();
        let server = server(bytes.clone(), false).await?;
        let mut pins = pins_for(&server.url, &bytes)?;
        if name == "safe" {
            artifact_mut(&mut pins.ytdlp).unpacked_size = 1;
        }
        let (progress, _rx) = watch::channel(Progress::Ready);
        let (_cancel, cancelled) = watch::channel(false);
        let error = install_with(
            &dir.tools(),
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        )
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("unsafe archive installed"))?;
        assert!(error.to_string().contains("archive"), "{error:#}");
        assert!(!dir.0.join("escaped").exists());
        assert_old_preserved(&dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn cancellation_leaves_the_previous_mode_and_files_intact() -> Result<()> {
    let dir = Fixture::new()?;
    old_install(&dir)?;
    let bytes = archive()?;
    let server = server(bytes.clone(), false).await?;
    let pins = pins_for(&server.url, &bytes)?;
    let (progress, _rx) = watch::channel(Progress::Ready);
    let (_cancel, cancelled) = watch::channel(true);
    assert!(
        install_with(
            &dir.tools(),
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled
        )
        .await
        .is_err()
    );
    assert_old_preserved(&dir)?;
    Ok(())
}

#[test]
fn startup_recovers_the_previous_install_after_a_crash_between_renames() -> Result<()> {
    let dir = Fixture::new()?;
    old_install(&dir)?;
    std::fs::rename(dir.tools(), dir.0.join("tools.old"))?;
    std::fs::create_dir(dir.0.join("tools.partial"))?;
    std::fs::write(dir.0.join("tools.partial/unverified"), b"partial")?;
    recover(&dir.tools())?;
    assert_old_preserved(&dir)?;
    assert!(!dir.0.join("tools.old").exists());
    Ok(())
}

#[tokio::test]
async fn cancelling_a_partial_response_cleans_staging_before_another_attempt() -> Result<()> {
    let dir = Fixture::new()?;
    old_install(&dir)?;
    let bytes = archive()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let pins = pins_for(&url, &bytes)?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await?;
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await?;
        socket
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", bytes.len()).as_bytes(),
            )
            .await?;
        socket.write_all(&bytes[..bytes.len() / 2]).await?;
        std::future::pending::<()>().await;
        Ok::<_, anyhow::Error>(())
    });
    let (progress, mut rx) = watch::channel(Progress::Ready);
    let (cancel, cancelled) = watch::channel(false);
    let directory = dir.tools();
    let install = tokio::spawn(async move {
        install_with(
            &directory,
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        rx.wait_for(|p| matches!(p, Progress::Downloading { bytes, .. } if *bytes > 0)),
    )
    .await??;
    cancel.send_replace(true);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), install)
            .await??
            .is_err()
    );
    server.abort();
    assert_old_preserved(&dir)?;
    Ok(())
}

#[test]
fn a_live_installer_lock_prevents_startup_from_deleting_its_partial_files() -> Result<()> {
    let dir = Fixture::new()?;
    let _guard = lock(&dir.tools())?;
    std::fs::create_dir(dir.0.join("tools.partial"))?;
    std::fs::write(dir.0.join("tools.partial/active"), b"still downloading")?;
    assert!(recover(&dir.tools()).is_err());
    assert!(dir.0.join("tools.partial/active").is_file());
    Ok(())
}

#[test]
fn symlink_entries_are_skipped_and_cannot_redirect_later_files() -> Result<()> {
    let dir = Fixture::new()?;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_symlink("link", "../outside", options)?;
    zip.start_file("link/safe", options)?;
    zip.write_all(b"inside")?;
    let bytes = zip.finish()?.into_inner();
    let size = u64::try_from(
        zip::ZipArchive::new(Cursor::new(&bytes))?
            .decompressed_size()
            .unwrap_or(0),
    )?;
    let file = dir.0.join("test.zip");
    std::fs::write(&file, bytes)?;
    let (_cancel, cancelled) = watch::channel(false);
    extract(&file, &dir.tools(), size, &cancelled)?;
    assert_eq!(std::fs::read(dir.tools().join("link/safe"))?, b"inside");
    assert!(!dir.0.join("outside").exists());
    Ok(())
}

#[tokio::test]
async fn publication_waits_for_running_extractors_and_can_be_cancelled_while_waiting() -> Result<()>
{
    let dir = Fixture::new()?;
    old_install(&dir)?;
    let bytes = archive()?;
    let server = server(bytes.clone(), false).await?;
    let pins = pins_for(&server.url, &bytes)?;
    let (progress, mut rx) = watch::channel(Progress::Ready);
    let (cancel, cancelled) = watch::channel(false);
    let execution = super::super::EXECUTIONS.read().await;
    let directory = dir.tools();
    let mut task = tokio::spawn(async move {
        install_with(
            &directory,
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        rx.wait_for(|p| *p == Progress::Checking),
    )
    .await??;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut task)
            .await
            .is_err(),
        "Published while an extractor was still running"
    );
    assert_eq!(
        std::fs::read(dir.tools().join("components.json"))?,
        b"previous marker"
    );
    cancel.send_replace(true);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), task)
            .await??
            .is_err()
    );
    drop(execution);
    assert_old_preserved(&dir)?;
    Ok(())
}

#[test]
fn execution_lock_fixture() -> Result<()> {
    let Some(directory) = std::env::var_os("GRAJEK_EXECUTION_LOCK_FIXTURE") else {
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    let lock = execution::open_lock(&directory)?;
    lock.lock_shared()?;
    std::fs::write(directory.with_extension("ready"), b"ready")?;
    let mut release = String::new();
    std::io::stdin().read_line(&mut release)?;
    Ok(())
}

#[tokio::test]
async fn publication_waits_for_a_managed_extractor_in_another_process() -> Result<()> {
    let dir = Fixture::new()?;
    old_install(&dir)?;
    let mut reader = tokio::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "media::components::download::tests::execution_lock_fixture",
            "--nocapture",
        ])
        .env("GRAJEK_EXECUTION_LOCK_FIXTURE", dir.tools())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !dir.tools().with_extension("ready").is_file() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    let bytes = archive()?;
    let server = server(bytes.clone(), false).await?;
    let pins = pins_for(&server.url, &bytes)?;
    let (progress, mut rx) = watch::channel(Progress::Ready);
    let (_cancel, cancelled) = watch::channel(false);
    let directory = dir.tools();
    let mut task = tokio::spawn(async move {
        install_with(
            &directory,
            &pins,
            reqwest::Client::new(),
            progress,
            cancelled,
        )
        .await
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        rx.wait_for(|p| *p == Progress::Checking),
    )
    .await??;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), &mut task)
            .await
            .is_err(),
        "Replaced another process's active tools"
    );
    reader
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("no lock-reader stdin"))?
        .write_all(b"release\n")
        .await?;
    assert!(reader.wait().await?.success());
    let _publication = tokio::time::timeout(Duration::from_secs(5), task).await???;
    assert!(
        dir.tools()
            .join("yt-dlp")
            .join(Tool::YtDlp.filename())
            .is_file()
    );
    Ok(())
}
