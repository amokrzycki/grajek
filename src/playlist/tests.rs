use super::*;
use anyhow::{Context as _, Result};
use serde_json::json;

#[test]
fn flat_fixture_preserves_valid_order_duplicates_and_optional_titles() -> Result<()> {
    let playlist = parse(include_bytes!("../../tests/fixtures/playlist.json"))?;
    assert_eq!(playlist.entries.len(), 4);
    assert_eq!(playlist.unavailable, 5);
    assert!(!playlist.limited);
    assert_eq!(playlist.entries[0].title.as_deref(), Some("First title"));
    assert_eq!(playlist.entries[0].url, playlist.entries[2].url);
    assert_eq!(
        playlist.entries[1].url,
        "https://www.youtube.com/watch?v=bbbbbbbbbbb"
    );
    assert_eq!(playlist.entries[1].title, None);
    assert_eq!(playlist.entries[3].title, None);
    Ok(())
}

#[test]
fn validates_structure_and_never_accepts_empty_or_nested_results() {
    for bytes in [
        b"null".as_slice(),
        b"[]",
        b"{}",
        b"{",
        br#"{"_type":"video","entries":[]}"#,
        br#"{"_type":"playlist","entries":{}}"#,
    ] {
        assert_eq!(parse(bytes), Err(Error::Malformed));
    }
    for entries in [
        json!([]),
        json!([null]),
        json!([{"id":"not-a-video-id"}]),
        json!([{"id":"aaaaaaaaaaa", "_type":"playlist"}]),
    ] {
        assert_eq!(
            parse(
                json!({"_type":"playlist", "entries":entries})
                    .to_string()
                    .as_bytes()
            ),
            Err(Error::Empty)
        );
    }
}

#[test]
fn scan_is_bounded_and_titles_use_existing_sanitization() -> Result<()> {
    let items: Vec<_> = (0..125)
        .map(|_| json!({"id":"aaaaaaaaaaa", "title":format!(" \u{0000}{}\n", "ż".repeat(1000))}))
        .collect();
    let playlist = parse(
        json!({"_type":"playlist", "entries":items})
            .to_string()
            .as_bytes(),
    )?;
    assert_eq!(playlist.entries.len(), 100);
    assert!(playlist.limited);
    assert_eq!(playlist.entries[0].title, Some("ż".repeat(200)));
    let one =
        parse(br#"{"_type":"playlist", "entries":[{"id":"aaaaaaaaaaa", "title":"  \n\t"}]}"#)?;
    assert_eq!(one.entries[0].title, None);
    assert!(!one.limited);
    Ok(())
}

fn shell(script: &str) -> Result<Child> {
    Ok(Command::new("sh")
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?)
}

#[tokio::test]
async fn subprocess_exit_timeout_and_both_output_bounds_are_enforced() -> Result<()> {
    assert_eq!(
        collect(shell("exit 3")?, TIMEOUT).await,
        Err(Error::Unavailable)
    );
    assert_eq!(
        collect(shell("exec sleep 30")?, Duration::from_millis(50)).await,
        Err(Error::Timeout)
    );
    for script in [
        "exec head -c 2097153 /dev/zero",
        "exec head -c 65537 /dev/zero >&2",
    ] {
        assert_eq!(
            collect(shell(script)?, TIMEOUT).await,
            Err(Error::OutputLimit)
        );
    }
    assert_eq!(collect(shell("printf ok")?, TIMEOUT).await?, b"ok");
    Ok(())
}

#[tokio::test]
async fn cancellation_kills_the_owned_subprocess() -> Result<()> {
    let child = shell("exec sleep 30")?;
    let pid = child.id().context("child has no pid")?;
    let task = tokio::spawn(collect(child, TIMEOUT));
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    timeout(Duration::from_secs(5), async {
        while std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            !stat
                .split_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with('Z'))
        }) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires public YouTube network access; metadata only, no audio or Discord"]
async fn public_playlist_metadata_only() -> Result<()> {
    let playlist =
        extract("https://www.youtube.com/playlist?list=PLBB231211A4F62143".into()).await?;
    assert_ne!(playlist.entries, Vec::<Entry>::new());
    assert!(playlist.entries.len() <= 100);
    assert!(
        playlist
            .entries
            .iter()
            .all(|entry| crate::config::video_url(&entry.url).is_ok_and(|url| url == entry.url))
    );
    assert!(playlist.entries.iter().any(|entry| entry.title.is_some()));
    Ok(())
}

#[test]
fn playlist_arguments_never_use_audio_or_single_video_extraction() {
    let args = args();
    for required in [
        "--ignore-config",
        "--flat-playlist",
        "--dump-single-json",
        "--playlist-end",
        "--lazy-playlist",
        "--skip-download",
    ] {
        assert!(args.contains(&required));
    }
    assert!(!args.contains(&"--no-playlist"));
    assert!(!args.contains(&"-f"));
    assert!(!args.iter().any(|arg| arg.contains("cookies")));
}

#[test]
fn declared_count_does_not_fabricate_a_scan_boundary() -> Result<()> {
    let playlist = parse(br#"{"_type":"playlist", "playlist_count":2, "entries":[{"id":"aaaaaaaaaaa", "title":"Available track"}]}"#)?;
    assert!(!playlist.limited, "only one source entry was inspected");
    let status = crate::player::ImportStatus::Finished {
        added: playlist.entries.len(),
        unavailable: playlist.unavailable,
        omitted: 0,
        limited: playlist.limited,
    };
    assert_eq!(status.message(), "Imported 1 track.");
    Ok(())
}
