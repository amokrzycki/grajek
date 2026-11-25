use crate::{
    media::components::{
        self, Mode, Need, Pins,
        channel::{Accepted, CheckError, fetch},
    },
    paths,
};
use gpui_kit::Context;

use super::PlayerApp;

const UNAVAILABLE: &str = "Couldn't check for updates. Try again later.";
const REJECTED: &str = "The update didn't pass verification and was ignored.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteCheck {
    Idle,
    Checking { generation: u64 },
    UpToDate,
    Available { ytdlp: String, deno: String },
    Failed(&'static str),
}

impl RemoteCheck {
    pub(super) fn status(&self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Checking { .. } => Some("Checking…".into()),
            Self::UpToDate => Some("Up to date".into()),
            Self::Available { ytdlp, .. } => Some(format!("yt-dlp {ytdlp} is available.")),
            Self::Failed(error) => Some((*error).into()),
        }
    }
}

pub(super) fn update_check_visible(mode: Mode, need: Option<Need>, system_override: bool) -> bool {
    !system_override && (mode == Mode::Managed || need == Some(Need::Missing))
}

fn outcome(
    result: Result<Accepted, CheckError>,
    need: Option<Need>,
    pins: anyhow::Result<Pins>,
) -> RemoteCheck {
    match result {
        Err(CheckError::Unavailable) => RemoteCheck::Failed(UNAVAILABLE),
        Err(CheckError::Rejected(_)) => RemoteCheck::Failed(REJECTED),
        Ok(_) => {
            if matches!(need, Some(Need::Update | Need::Missing | Need::Repair)) {
                match pins {
                    Ok(pins) => RemoteCheck::Available {
                        ytdlp: pins.ytdlp.version,
                        deno: pins.deno.version,
                    },
                    Err(_) => RemoteCheck::Failed(UNAVAILABLE),
                }
            } else {
                RemoteCheck::UpToDate
            }
        }
    }
}

impl PlayerApp {
    pub(in crate::app) fn check_component_updates(&mut self, cx: &mut Context<Self>) {
        if self.components.busy() || matches!(self.components.remote, RemoteCheck::Checking { .. })
        {
            return;
        }
        let Some(runtime) = self.components.runtime.clone() else {
            return;
        };
        let Some(directory) = paths::component_manifest_dir() else {
            self.components.remote = RemoteCheck::Failed(UNAVAILABLE);
            cx.notify();
            return;
        };
        self.components.remote_generation = self.components.remote_generation.wrapping_add(1);
        let generation = self.components.remote_generation;
        self.components.remote = RemoteCheck::Checking { generation };
        let job = runtime.spawn(async move {
            let client = fetch::client().map_err(|_| CheckError::Unavailable)?;
            fetch::check(&client, fetch::MANIFEST_BASE, &directory).await
        });
        cx.spawn(async move |entity, cx| {
            let result = job.await.unwrap_or(Err(CheckError::Unavailable));
            let _ = entity.update(cx, |this, cx| {
                this.finish_component_update_check(generation, result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(in crate::app) fn finish_component_update_check(
        &mut self,
        generation: u64,
        result: Result<Accepted, CheckError>,
    ) {
        if self.components.remote != (RemoteCheck::Checking { generation }) {
            return;
        }
        self.components.remote = outcome(
            result,
            components::tools().need(),
            components::effective_pins(paths::component_manifest_dir().as_deref()),
        );
    }
}

#[cfg(test)]
mod tests;
