//! Shared bounded `yt-dlp` process handling and flat-playlist metadata rules.

use serde_json::Value;
use std::process::Stdio;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::{Child, Command},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Tool,
    Unavailable,
    OutputLimit,
}

async fn read_bounded(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| Failure::Tool)?;
    if bytes.len() > limit {
        return Err(Failure::OutputLimit);
    }
    Ok(bytes)
}

pub(crate) async fn read_output(
    child: &mut Child,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Vec<u8>, Failure> {
    let stdout = child.stdout.take().ok_or(Failure::Tool)?;
    let stderr = child.stderr.take().ok_or(Failure::Tool)?;
    let (output, _diagnostics, status) = tokio::try_join!(
        read_bounded(stdout, stdout_limit),
        read_bounded(stderr, stderr_limit),
        async { child.wait().await.map_err(|_| Failure::Tool) },
    )?;
    if !status.success() {
        return Err(Failure::Unavailable);
    }
    Ok(output)
}

pub(crate) fn flat_args(end: usize, yes_playlist: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "--ignore-config",
        "--no-plugin-dirs",
        "--flat-playlist",
        "--dump-single-json",
        "--playlist-end",
    ]
    .map(str::to_owned)
    .to_vec();
    args.push(end.to_string());
    args.extend(["--lazy-playlist", "--skip-download"].map(str::to_owned));
    if yes_playlist {
        args.push("--yes-playlist".into());
    }
    args.extend(NETWORK_ARGS.map(str::to_owned));
    args.extend(
        [
            "--no-warnings",
            "--no-progress",
            // End options before the caller-appended URL.
            "--",
        ]
        .map(str::to_owned),
    );
    args
}

/// Network limits every yt-dlp invocation shares, in the order they reach the command line.
pub(crate) const NETWORK_ARGS: [&str; 6] = [
    "--socket-timeout",
    "15",
    "--retries",
    "0",
    "--extractor-retries",
    "0",
];

/// No stdin, captured output, killed with the future.
pub(crate) fn spawn(mut command: Command) -> Command {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub(crate) fn unavailable_entry(entry: &Value) -> bool {
    entry
        .get("_type")
        .is_some_and(|value| !matches!(value.as_str(), Some("url" | "url_transparent" | "video")))
        || entry
            .get("availability")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                matches!(
                    value,
                    "private" | "premium_only" | "subscriber_only" | "needs_auth" | "unavailable"
                )
            })
        || entry
            .get("title")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                matches!(
                    value,
                    "[Deleted video]" | "[Private video]" | "[Unavailable video]"
                )
            })
        || entry
            .get("ie_key")
            .is_some_and(|value| value.as_str() != Some("Youtube"))
}

pub(crate) fn playlist_entries(root: &Value) -> Option<&[Value]> {
    if root.get("_type").and_then(Value::as_str) != Some("playlist") {
        return None;
    }
    root.get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::{flat_args, unavailable_entry};
    use serde_json::json;

    #[test]
    fn flat_args_keep_the_documented_order_and_shared_network_limits() {
        assert_eq!(
            flat_args(5, false),
            [
                "--ignore-config",
                "--no-plugin-dirs",
                "--flat-playlist",
                "--dump-single-json",
                "--playlist-end",
                "5",
                "--lazy-playlist",
                "--skip-download",
                "--socket-timeout",
                "15",
                "--retries",
                "0",
                "--extractor-retries",
                "0",
                "--no-warnings",
                "--no-progress",
                "--",
            ]
        );
        let playlist = flat_args(5, true);
        assert_eq!(playlist[8], "--yes-playlist");
        assert_eq!(&playlist[9..], &flat_args(5, false)[8..]);
    }

    #[test]
    fn shared_unavailability_rules_match_every_flat_entry_condition() {
        for entry in [
            json!({"_type": "channel"}),
            json!({"availability": "private"}),
            json!({"availability": "premium_only"}),
            json!({"availability": "subscriber_only"}),
            json!({"availability": "needs_auth"}),
            json!({"availability": "unavailable"}),
            json!({"title": "[Deleted video]"}),
            json!({"title": "[Private video]"}),
            json!({"title": "[Unavailable video]"}),
            json!({"ie_key": "Vimeo"}),
        ] {
            assert!(unavailable_entry(&entry), "accepted {entry}");
        }
        for entry in [
            json!({"id": "BaW_jenozKc"}),
            json!({"_type": "url_transparent", "ie_key": "Youtube", "title": "Song"}),
            json!({"_type": "video", "availability": "public"}),
        ] {
            assert!(!unavailable_entry(&entry), "rejected {entry}");
        }
    }
}
