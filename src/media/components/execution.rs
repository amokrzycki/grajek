use super::{EXECUTIONS, Tools, installed_versions, tools};
use anyhow::{Context as _, Result};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
    time::Duration,
};

pub(crate) struct Execution {
    pub tools: Tools,
    _process: tokio::sync::RwLockReadGuard<'static, ()>,
    _resources: Option<File>,
}

pub(super) fn open_lock(directory: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.with_extension("execution.lock"))?)
}

pub(super) async fn lock(directory: &Path, shared: bool) -> Result<File> {
    let file = open_lock(directory)?;
    loop {
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match result {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(TryLockError::Error(error)) => {
                return Err(error).context("Couldn't lock playback component resources.");
            }
        }
    }
}

pub(crate) async fn acquire() -> Result<Execution> {
    let process = EXECUTIONS.read().await;
    let mut tools = tools();
    let resources = if let Some(directory) = &tools.directory {
        let file = lock(directory, true).await?;
        // Another instance may have updated the same directory while this instance was idle.
        tools.versions = installed_versions(directory);
        Some(file)
    } else {
        None
    };
    Ok(Execution {
        tools,
        _process: process,
        _resources: resources,
    })
}
