use super::components::{self, execution::Execution, status::ComponentState};
use anyhow::{Result, ensure};
use std::time::Duration;
use tokio::{process::Command, time::timeout};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    YtDlp,
    Deno,
}
impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Deno => "deno",
        }
    }
    /// Name a PATH installation uses; the managed Linux artifact is named differently.
    pub(crate) fn system_name(self) -> &'static str {
        #[cfg(target_os = "linux")]
        {
            self.name()
        }
        #[cfg(target_os = "windows")]
        {
            self.filename()
        }
    }
    pub(crate) fn filename(self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Self::YtDlp => "yt-dlp_linux",
            #[cfg(target_os = "linux")]
            Self::Deno => "deno",
            #[cfg(target_os = "windows")]
            Self::YtDlp => "yt-dlp.exe",
            #[cfg(target_os = "windows")]
            Self::Deno => "deno.exe",
        }
    }
}

pub async fn check_tools() -> Result<()> {
    let execution = timeout(Duration::from_secs(5), components::execution::acquire())
        .await
        .map_err(|_| components::tools().failure())?
        .map_err(|_| components::tools().failure())?;
    ensure!(
        execution
            .tools
            .probe()
            .await
            .iter()
            .all(|report| report.state == ComponentState::Ready),
        execution.tools.failure()
    );
    drop(execution);
    Ok(())
}

/// Keep the returned guard alive for the whole run: it pins the binaries the command was
/// built from.
pub(crate) async fn ytdlp_command() -> Result<(Execution, Command)> {
    let execution = components::execution::acquire().await?;
    let command = execution.tools.command(Tool::YtDlp);
    Ok((execution, command))
}

pub(crate) fn supported_deno(output: &str) -> bool {
    let Some(version) = output
        .strip_prefix("deno ")
        .and_then(|s| s.split_whitespace().next())
    else {
        return false;
    };
    let mut parts = version.split('.');
    match (
        parts.next().and_then(|s| s.parse::<u32>().ok()),
        parts.next().and_then(|s| s.parse::<u32>().ok()),
    ) {
        (Some(major), Some(minor)) => major > 2 || (major == 2 && minor >= 3),
        _ => false,
    }
}
