use std::{path::PathBuf, process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt as _, process::Command};

use super::{Mode, Tools};
use crate::media::Tool;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ComponentState {
    NotChecked,
    Checking,
    Missing,
    Ready,
    NeedsRepair(String),
}

impl ComponentState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotChecked => "Not checked",
            Self::Checking => "Checking…",
            Self::Missing => "Missing",
            Self::Ready => "Ready",
            Self::NeedsRepair(_) => "Needs repair",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ComponentReport {
    pub tool: Tool,
    pub state: ComponentState,
    pub version: Option<String>,
}

impl Tools {
    pub fn component_path(&self, tool: Tool) -> Option<PathBuf> {
        if self.mode == Mode::Managed {
            return self.path(tool).is_file().then(|| self.path(tool));
        }
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(tool.system_name()))
                .find(|path| path.is_file())
        })
    }

    pub fn inspect(&self) -> Vec<ComponentReport> {
        [Tool::YtDlp, Tool::Deno]
            .into_iter()
            .map(|tool| ComponentReport {
                tool,
                state: if self.component_path(tool).is_some() {
                    ComponentState::NotChecked
                } else {
                    ComponentState::Missing
                },
                version: None,
            })
            .collect()
    }

    pub async fn check(&self) -> Vec<ComponentReport> {
        let Ok(Ok(execution)) = tokio::time::timeout(
            Duration::from_secs(5),
            super::execution::acquire_tools(self.clone()),
        )
        .await
        else {
            let mut reports = self.inspect();
            for report in &mut reports {
                report.state = ComponentState::NeedsRepair("Components are busy or couldn't be locked. Try checking again after other Grajek instances finish.".into());
            }
            return reports;
        };
        let reports = execution.tools.probe().await;
        drop(execution);
        reports
    }

    /// The caller holds execution guards, or owns an unpublished staging directory.
    pub(crate) async fn probe(&self) -> Vec<ComponentReport> {
        let mut reports = self.inspect();
        for report in &mut reports {
            if report.state != ComponentState::Missing {
                let mut command = self.command(report.tool);
                command.arg("--version");
                *report = check_command(self, report.tool, command).await;
            }
        }
        reports
    }
}

async fn check_command(tools: &Tools, tool: Tool, mut command: Command) -> ComponentReport {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(
                |_| "Couldn't run this component. Download it again or check file permissions.",
            )?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Couldn't read the component version.")?;
        let mut bytes = Vec::new();
        stdout
            .take(4097)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "Couldn't read the component version.")?;
        if bytes.len() > 4096 {
            return Err("The component returned too much version information.");
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "The component check failed.")?;
        if !status.success() {
            return Err("The component couldn't complete its version check.");
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })
    .await;
    let (state, version) = match result {
        Ok(Ok(output)) => {
            let version = output
                .lines()
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let state = if tools.valid_version(tool, &output) {
                ComponentState::Ready
            } else {
                ComponentState::NeedsRepair("Unsupported version or damaged installation. Download the verified components.".into())
            };
            (state, version)
        }
        Ok(Err(reason)) => (ComponentState::NeedsRepair(reason.into()), None),
        Err(_) => (
            ComponentState::NeedsRepair(
                "The version check timed out. Try checking again or repair this component.".into(),
            ),
            None,
        ),
    };
    ComponentReport {
        tool,
        state,
        version,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn checking_never_waits_forever_for_a_publication_lock() {
        let _guard = super::super::EXECUTIONS.write().await;
        let tools = Tools::select(true, None);
        let reports = tokio::time::timeout(Duration::from_secs(6), tools.check()).await;
        assert!(
            reports.is_ok(),
            "The UI check must time out before six seconds"
        );
        if let Ok(reports) = reports {
            assert!(
                reports
                    .iter()
                    .all(|r| matches!(r.state, ComponentState::NeedsRepair(_)))
            );
        }
    }

    fn output_command(output: &str, exit: u8) -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd.exe");
            command.args(["/d", "/c", &format!("echo({output}& exit /b {exit}")]);
            command.creation_flags(0x0800_0000);
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", &format!("printf '%s\\n' '{output}'; exit {exit}")]);
            command
        }
    }

    #[tokio::test]
    async fn ready_requires_success_and_supported_version() {
        let tools = Tools::select(true, None);
        for (tool, output, exit, ready) in [
            (Tool::YtDlp, "2026.10.26", 0, true),
            (Tool::Deno, "deno 2.9.8 (stable)", 0, true),
            (Tool::Deno, "deno 2.2.0", 0, false),
            (Tool::Deno, "other 2.9.8", 0, false),
            (Tool::YtDlp, "2026.10.26", 1, false),
            (Tool::YtDlp, "", 0, false),
        ] {
            let report = check_command(&tools, tool, output_command(output, exit)).await;
            assert_eq!(
                report.state == ComponentState::Ready,
                ready,
                "{output} / {exit}"
            );
        }
    }
}
