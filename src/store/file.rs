use std::{
    fs::{self, DirBuilder, File},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::saved::Saved;
use crate::{favorites::FAVORITES_LIMIT, paths};

pub struct Loaded {
    pub saved: Saved,
    /// `None` disables saving, e.g. when no data directory exists or a damaged file
    /// could not be set aside.
    pub path: Option<PathBuf>,
    pub warning: Option<String>,
}

pub fn open() -> Loaded {
    let dir = paths::app_data_dir();
    load(dir.map(|dir| dir.join("state.json")))
}

pub fn load(path: Option<PathBuf>) -> Loaded {
    let Some(file) = path else {
        return Loaded {
            saved: Saved::default(),
            path: None,
            warning: Some(
                "No data directory found; local data and settings will not be saved.".into(),
            ),
        };
    };
    let text = match fs::read(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                saved: Saved::default(),
                path: Some(file),
                warning: None,
            };
        }
        Err(error) => {
            tracing::warn!(%error, path = %file.display(), "Could not read saved state");
            return Loaded {
                saved: Saved::default(),
                path: None,
                warning: Some(format!(
                    "Could not read {}; local data and settings will not be saved.",
                    file.display()
                )),
            };
        }
    };
    match serde_json::from_slice::<Saved>(&text) {
        Ok(saved) => {
            let (saved, dropped) = saved.sanitized();
            if dropped == 0 {
                return Loaded {
                    saved,
                    path: Some(file),
                    warning: None,
                };
            }
            match backup_exact(&file, &text) {
                Ok(backup) => Loaded {
                    saved,
                    path: Some(file),
                    warning: Some(format!(
                        "The favorites list was too large; kept the first {FAVORITES_LIMIT} favorites and backed up the original to {}.",
                        backup.display()
                    )),
                },
                Err(error) => {
                    tracing::warn!(%error, path = %file.display(), "Could not back up oversized favorites");
                    Loaded {
                        saved,
                        path: None,
                        warning: Some(format!(
                            "The favorites list was too large and could not be backed up; the file at {} was left unchanged and saving is disabled this session.",
                            file.display()
                        )),
                    }
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, path = %file.display(), "Saved state is malformed");
            // Keep the damaged file for inspection; the next save must not replace it.
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let backup = file.with_extension(format!("json.corrupt-{seconds}"));
            let kept = fs::rename(&file, &backup).is_ok();
            let warning = if kept {
                format!(
                    "Saved history was unreadable; starting empty. The old file is kept as {}.",
                    backup.display()
                )
            } else {
                format!(
                    "Saved history at {} is unreadable and could not be set aside; it will not be overwritten.",
                    file.display()
                )
            };
            Loaded {
                saved: Saved::default(),
                path: kept.then_some(file),
                warning: Some(warning),
            }
        }
    }
}

/// Copies the original file bytes to a distinct, previously-unused name in the same
/// directory, so an oversized-favorites recovery never overwrites an earlier backup.
fn backup_exact(original: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    loop {
        let backup = original.with_extension(format!(
            "json.favorites-{seconds}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match File::create_new(&backup) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                return Ok(backup);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

struct Temporary(PathBuf);

impl Temporary {
    fn create(path: &Path) -> io::Result<(Self, File)> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let temporary = path.with_extension(format!(
                "json.tmp-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match File::create_new(&temporary) {
                Ok(file) => return Ok((Self(temporary), file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Writes through a private temporary file in the same directory and renames it into place, so a
/// crash leaves either the old file or the new one.
pub fn write(path: &Path, saved: &Saved) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("state path has no directory"))?;
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let (temporary, mut file) = Temporary::create(path)?;
    file.write_all(&serde_json::to_vec_pretty(saved)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary.0, path)?;
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}
