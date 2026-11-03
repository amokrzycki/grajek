use std::{
    fs::{File, OpenOptions},
    io::Read as _,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};
use tokio::{io::AsyncWriteExt as _, sync::watch, task::JoinHandle};

use super::{Artifact, Mode, Pins, Tools};
use crate::media::Tool;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Progress {
    #[default]
    Ready,
    Downloading {
        bytes: u64,
        total: u64,
    },
    Extracting,
    Checking,
    Complete,
    Cancelled,
    Failed(String),
}

pub(crate) struct Download {
    pub progress: watch::Receiver<Progress>,
    cancel: watch::Sender<bool>,
    _task: JoinHandle<()>,
}

impl Download {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn start(runtime: &tokio::runtime::Handle, directory: PathBuf) -> Result<Download> {
    let pins = super::pins()?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(15))
        .build()?;
    let (progress, updates) = watch::channel(Progress::Ready);
    let (cancel, cancelled) = watch::channel(false);
    let task = runtime.spawn(async move {
        let result = install_with(
            &directory,
            &pins,
            client,
            progress.clone(),
            cancelled.clone(),
        )
        .await;
        let status = match result {
            Ok(()) => {
                super::refresh(directory);
                Progress::Complete
            }
            Err(_) if *cancelled.borrow() || cancelled.has_changed().is_err() => {
                Progress::Cancelled
            }
            Err(error) => {
                tracing::warn!(%error, "Playback components download failed");
                Progress::Failed(format!("{error:#}"))
            }
        };
        progress.send_replace(status);
    });
    Ok(Download {
        progress: updates,
        cancel,
        _task: task,
    })
}

fn lock(directory: &Path) -> Result<File> {
    let parent = directory
        .parent()
        .context("Playback components directory has no parent.")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Couldn't create {}", parent.display()))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.with_extension("lock"))?;
    file.try_lock().context(
        "Another Grajek instance is installing playback components. Try again when it finishes.",
    )?;
    Ok(file)
}

fn remove(directory: &Path) -> Result<()> {
    match std::fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn recover_locked(directory: &Path) -> Result<()> {
    let old = directory.with_extension("old");
    if old.try_exists()? {
        if directory.join("components.json").is_file() {
            remove(&old)?;
        } else {
            remove(directory)?;
            std::fs::rename(&old, directory)?;
        }
    }
    remove(&directory.with_extension("partial"))
}

pub(crate) fn recover(directory: &Path) -> Result<()> {
    if directory.parent().is_none_or(|parent| !parent.exists()) {
        return Ok(());
    }
    let _lock = lock(directory)?;
    recover_locked(directory)
}

fn check_cancel(cancel: &watch::Receiver<bool>) -> Result<()> {
    ensure!(
        !*cancel.borrow() && cancel.has_changed().is_ok(),
        "Download cancelled."
    );
    Ok(())
}

async fn download(
    client: &reqwest::Client,
    pin: &Artifact,
    destination: &Path,
    completed: u64,
    total: u64,
    progress: &watch::Sender<Progress>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<()> {
    check_cancel(cancel)?;
    let mut response = tokio::select! {
        _ = cancel.changed() => anyhow::bail!("Download cancelled."),
        response = client.get(&pin.url).send() => response.map_err(reqwest::Error::without_url)
            .context("Couldn't download playback components. Check your connection and try again.")?
            .error_for_status().map_err(reqwest::Error::without_url)?,
    };
    let mut file = tokio::fs::File::create_new(destination).await?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    loop {
        check_cancel(cancel)?;
        let chunk = tokio::select! {
            _ = cancel.changed() => anyhow::bail!("Download cancelled."),
            chunk = response.chunk() => chunk.map_err(reqwest::Error::without_url)
                .context("Couldn't download playback components. Check your connection and try again.")?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        bytes = bytes
            .checked_add(u64::try_from(chunk.len())?)
            .context("Download size overflow.")?;
        ensure!(
            bytes <= pin.size,
            "The downloaded file exceeds its expected size. Nothing was installed."
        );
        hash.update(&chunk);
        file.write_all(&chunk).await?;
        progress.send_replace(Progress::Downloading {
            bytes: completed + bytes,
            total,
        });
    }
    ensure!(
        bytes == pin.size && format!("{:x}", hash.finalize()) == pin.sha256,
        "The downloaded file doesn't match the expected checksum. Nothing was installed."
    );
    file.sync_all().await?;
    Ok(())
}

fn extract(
    archive: &Path,
    directory: &Path,
    limit: u64,
    cancel: &watch::Receiver<bool>,
) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut total = 0_u64;
    for index in 0..zip.len() {
        check_cancel(cancel)?;
        let mut entry = zip.by_index(index)?;
        let name = entry
            .enclosed_name()
            .context("Unsafe archive path. Nothing was installed.")?;
        total = total
            .checked_add(entry.size())
            .context("Invalid archive size.")?;
        ensure!(
            total <= limit,
            "The archive exceeds its expected unpacked size."
        );
        if entry.is_symlink() {
            continue;
        }
        let path = directory.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        std::fs::create_dir_all(path.parent().context("Invalid archive path.")?)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let size = entry.size();
        let copied = std::io::copy(&mut entry.by_ref().take(size.saturating_add(1)), &mut file)?;
        ensure!(copied == size, "The archive has an invalid entry size.");
        if let Some(mode) = entry.unix_mode() {
            file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    ensure!(
        total == limit,
        "The archive doesn't match its expected unpacked size."
    );
    Ok(())
}

fn publish(partial: &Path, directory: &Path) -> Result<()> {
    let old = directory.with_extension("old");
    let previous = directory.try_exists()?;
    if previous {
        std::fs::rename(directory, &old)?;
    }
    if let Err(error) = std::fs::rename(partial, directory) {
        if previous {
            std::fs::rename(&old, directory)
                .context("Couldn't restore the previous playback components.")?;
        }
        return Err(error.into());
    }
    // A failed old-directory cleanup must not report a successful install as failed.
    if let Err(error) = remove(&old) {
        tracing::warn!(%error, "Previous components will be cleaned on next start");
    }
    Ok(())
}

async fn install_with(
    directory: &Path,
    pins: &Pins,
    client: reqwest::Client,
    progress: watch::Sender<Progress>,
    mut cancel: watch::Receiver<bool>,
) -> Result<()> {
    ensure!(
        directory.is_absolute(),
        "Playback components require an absolute directory."
    );
    let _lock = lock(directory)?;
    recover_locked(directory)?;
    let partial = directory.with_extension("partial");
    std::fs::create_dir(&partial)?;
    let result = async {
        let total = pins.ytdlp.linux.size.checked_add(pins.deno.linux.size).context("Invalid component sizes.")?;
        let mut completed = 0;
        for (tool, pin) in [(Tool::YtDlp, &pins.ytdlp.linux), (Tool::Deno, &pins.deno.linux)] {
            let archive = partial.join(format!("{}.zip", tool.name()));
            download(&client, pin, &archive, completed, total, &progress, &mut cancel).await?;
            progress.send_replace(Progress::Extracting);
            let destination = partial.join(tool.name());
            let uncompressed = pin.unpacked_size;
            let cancelled = cancel.clone();
            tokio::task::spawn_blocking(move || extract(&archive, &destination, uncompressed, &cancelled)).await??;
            tokio::fs::remove_file(partial.join(format!("{}.zip", tool.name()))).await?;
            completed += pin.size;
        }
        check_cancel(&cancel)?;
        progress.send_replace(Progress::Checking);
        let tools = Tools { mode: Mode::Managed, directory: Some(partial.clone()), versions: Some(pins.versions()) };
        for tool in [Tool::YtDlp, Tool::Deno] {
            let output = tokio::time::timeout(Duration::from_secs(5), tools.command(tool).arg("--version").kill_on_drop(true).output()).await??;
            ensure!(output.status.success() && tools.valid_version(tool, &String::from_utf8_lossy(&output.stdout)),
                "Downloaded playback components couldn't run or have an unexpected version. Check system compatibility.");
        }
        check_cancel(&cancel)?;
        std::fs::write(partial.join("components.json"), serde_json::to_vec(&pins.versions())?)?;
        // No await/cancellation point between the two renames; startup restores tools.old after a crash.
        publish(&partial, directory)?;
        Ok(())
    }.await;
    if result.is_err() {
        remove(&partial)?;
    }
    result
}

#[cfg(test)]
mod tests;
