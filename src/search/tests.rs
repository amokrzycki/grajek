use super::*;
use anyhow::Context as _;
use serde_json::json;
use std::{io::Write as _, process::Stdio};
use tokio::{process::Command, sync::watch, time::timeout};

#[test]
fn fixture_normalizes_metadata_and_keeps_only_factual_fields() -> anyhow::Result<()> {
    let results = parse(include_bytes!("../../tests/fixtures/search.json"))
        .context("parse the sanitized search fixture")?;
    assert_eq!(results.items.len(), 4);
    assert_eq!(results.rejected, 0);
    assert_eq!(results.items[0].title.as_deref(), Some("Bach Suite No. 1"));
    assert_eq!(results.items[0].channel.as_deref(), Some("Artist Channel"));
    assert_eq!(
        results.items[0].duration,
        Some(Duration::from_millis(123_500))
    );
    assert_eq!(results.items[1].channel, None);
    assert_eq!(results.items[1].duration, None);
    assert_eq!(results.items[2].duration, None);
    assert!(results.items[2].live);
    assert_eq!(results.items[3].id(), "dQw4w9WgXcQ");
    Ok(())
}

#[test]
fn query_normalizes_and_rejects_empty_control_and_oversized_input() -> anyhow::Result<()> {
    assert_eq!(
        query("  Bach\n cello  ").context("normalize a valid query")?,
        "Bach cello"
    );
    assert!(matches!(query(" \t\n "), Err(Error::InvalidQuery)));
    assert!(matches!(
        query("Bach\u{0007}cello"),
        Err(Error::InvalidQuery)
    ));
    assert!(matches!(query(&"a".repeat(201)), Err(Error::InvalidQuery)));
    assert_eq!(
        query(&"雪".repeat(200)).context("accept 200 Unicode scalars")?,
        "雪".repeat(200)
    );
    Ok(())
}

#[test]
fn parser_rejects_bad_roots_and_distinguishes_empty_from_filtered() -> anyhow::Result<()> {
    for bytes in [
        b"null".as_slice(),
        b"[]",
        b"{}",
        b"{",
        br#"{"_type":"video","entries":[]}"#,
    ] {
        assert_eq!(parse(bytes), Err(Error::Malformed));
    }
    let empty = parse(br#"{"_type":"playlist","entries":[]}"#)
        .context("parse a successful empty search")?;
    assert_eq!(empty.items, Vec::<ResultItem>::new());
    let results = parse(br#"{"_type":"playlist","entries":[null,{"id":"invalid"}]}"#)
        .context("parse filtered search candidates")?;
    assert_eq!(results.items, Vec::<ResultItem>::new());
    assert_eq!(results.rejected, 2);
    Ok(())
}

#[test]
fn parser_rejects_conflicts_overflow_and_caps_distinct_results() -> anyhow::Result<()> {
    let conflicts = json!({"_type":"playlist","entries":[
        {"id":"aqz-KE-bpKQ","ie_key":"Vimeo"},
        {"id":"aqz-KE-bpKQ","url":"https://www.youtube.com/watch?v=M7lc1UVf-VE"},
        {"id":"aqz-KE-bpKQ","webpage_url":"https://www.youtube.com/watch?v=M7lc1UVf-VE"},
        {"id":"aqz-KE-bpKQ","url":"https://youtu.be/M7lc1UVf-VE"},
        {"id":"aqz-KE-bpKQ","ie_key":"Youtube","url":"https://www.youtube.com/watch?v=aqz-KE-bpKQ"}
    ]});
    let results =
        parse(conflicts.to_string().as_bytes()).context("parse conflicting candidates")?;
    assert_eq!(results.rejected, 4);
    assert_eq!(results.items.len(), 1);
    let too_many = json!({"_type":"playlist","entries":(0..6).map(|_| json!({"id":"aqz-KE-bpKQ"})).collect::<Vec<_>>()});
    assert_eq!(
        parse(too_many.to_string().as_bytes()),
        Err(Error::Malformed)
    );
    Ok(())
}

#[test]
fn oversized_display_values_become_unavailable_without_truncation() -> anyhow::Result<()> {
    let value = json!({"_type":"playlist","entries":[{
        "id":"aqz-KE-bpKQ","title":"x".repeat(4097),"channel":"c".repeat(513)
    }]});
    let results = parse(value.to_string().as_bytes()).context("parse oversized display fields")?;
    assert_eq!(results.items[0].title, None);
    assert_eq!(results.items[0].channel, None);
    Ok(())
}

#[test]
fn parser_skips_unavailable_and_deduplicates_canonical_video_ids() -> anyhow::Result<()> {
    let value = json!({"_type":"playlist","entries":[
        {"id":"aqz-KE-bpKQ","ie_key":"Youtube","url":"https://i.ytimg.com/thumbnail.jpg"},
        {"id":"aqz-KE-bpKQ","ie_key":"Youtube","availability":"private"},
        {"id":"aqz-KE-bpKQ","ie_key":"Youtube"}
    ]});
    let results = parse(value.to_string().as_bytes()).context("parse unavailable duplicates")?;
    assert_eq!(results.items.len(), 1);
    assert_eq!(results.items[0].id(), "aqz-KE-bpKQ");
    assert_eq!(results.rejected, 2);
    Ok(())
}

#[test]
fn duration_accepts_representable_positive_values_without_millisecond_truncation()
-> anyhow::Result<()> {
    let value = json!({"_type":"playlist","entries":[
        {"id":"aqz-KE-bpKQ","duration":0.0001},
        {"id":"M7lc1UVf-VE","duration":10_000_000_000.0},
        {"id":"ScMzIvxBSi4","duration":0.0},
        {"id":"dQw4w9WgXcQ","duration":1e100}
    ]});
    let items = parse(value.to_string().as_bytes())
        .context("parse duration metadata")?
        .items;
    assert_eq!(items[0].duration, Some(Duration::from_micros(100)));
    assert_eq!(items[1].duration, Some(Duration::from_secs(10_000_000_000)));
    assert_eq!(items[2].duration, None);
    assert_eq!(items[3].duration, None);
    Ok(())
}

fn fixture(mode: &str, marker: Option<&std::path::Path>) -> std::io::Result<Child> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "search::tests::subprocess_fixture",
            "--nocapture",
        ])
        .env("GRAJEK_SEARCH_FIXTURE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(marker) = marker {
        command.env("GRAJEK_SEARCH_MARKER", marker);
        command.env("GRAJEK_SEARCH_LOCK", marker.with_extension("lock"));
    }
    command.spawn()
}

#[test]
fn subprocess_fixture() -> anyhow::Result<()> {
    let Ok(mode) = std::env::var("GRAJEK_SEARCH_FIXTURE") else {
        return Ok(());
    };
    match mode.as_str() {
        "exit" => std::process::exit(3),
        "wait" => {
            let marker = std::env::var("GRAJEK_SEARCH_MARKER")?;
            let mut file = std::fs::File::create(marker)?;
            write!(file, "{}", std::process::id())?;
            file.flush()?;
            let lock = std::env::var("GRAJEK_SEARCH_LOCK")?;
            let lock = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock)?;
            lock.lock()?;
            loop {
                std::thread::park();
            }
        }
        "stdout" => std::io::stdout().write_all(&vec![0; STDOUT_LIMIT + 1])?,
        "stderr" => std::io::stderr().write_all(&vec![0; STDERR_LIMIT + 1])?,
        "plausible-error" => {
            std::io::stdout().write_all(br#"{"_type":"playlist","entries":[]}"#)?;
            std::process::exit(3);
        }
        "ok" => std::io::stdout().write_all(br#"{"_type":"playlist","entries":[]}"#)?,
        _ => anyhow::bail!("unknown search process fixture"),
    }
    std::process::exit(0)
}

fn marker_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "grajek-search-{}-{}.marker",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ))
}

async fn wait_marker(path: &std::path::Path, lock: &std::path::Path) -> anyhow::Result<u32> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(path)
                && let Ok(pid) = value.parse()
                && std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(lock)
                    .is_ok_and(|file| file.try_lock().is_err())
            {
                return Ok(pid);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?
}

async fn assert_released(pid: u32, lock: &std::path::Path) -> anyhow::Result<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            let lock_released = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)
                .is_ok_and(|file| file.try_lock().is_ok());
            let process_reaped = !cfg!(target_os = "linux")
                || !std::path::Path::new(&format!("/proc/{pid}")).exists();
            if lock_released && process_reaped {
                break Ok::<(), anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    Ok(())
}

async fn assert_lock_released(lock: &std::path::Path) -> anyhow::Result<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock)
                .is_ok_and(|file| file.try_lock().is_ok())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn child_exit_and_both_pipe_caps_are_enforced() -> anyhow::Result<()> {
    let (_tx, rx) = watch::channel(false);
    assert_eq!(
        collect(
            fixture("exit", None)?,
            rx.clone(),
            TIMEOUT,
            STDOUT_LIMIT,
            STDERR_LIMIT
        )
        .await,
        Err(Error::Unavailable)
    );
    assert_eq!(
        collect(
            fixture("plausible-error", None)?,
            rx.clone(),
            TIMEOUT,
            STDOUT_LIMIT,
            STDERR_LIMIT
        )
        .await,
        Err(Error::Unavailable)
    );
    for mode in ["stdout", "stderr"] {
        assert_eq!(
            collect(
                fixture(mode, None)?,
                rx.clone(),
                TIMEOUT,
                STDOUT_LIMIT,
                STDERR_LIMIT
            )
            .await,
            Err(Error::OutputLimit)
        );
    }
    let output = collect(
        fixture("ok", None)?,
        rx,
        TIMEOUT,
        STDOUT_LIMIT,
        STDERR_LIMIT,
    )
    .await?;
    anyhow::ensure!(
        output.ends_with(br#"{"_type":"playlist","entries":[]}"#),
        "fixture emits its complete metadata after the test harness header"
    );
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::print_stderr,
    reason = "The process-lifecycle regression retains PID and cleanup timing evidence"
)]
async fn explicit_cancel_kills_waits_and_releases_child_resources() -> anyhow::Result<()> {
    let marker = marker_path();
    let lock = marker.with_extension("lock");
    let child = fixture("wait", Some(&marker))?;
    let (tx, rx) = watch::channel(false);
    let task = tokio::spawn(collect(child, rx, TIMEOUT, STDOUT_LIMIT, STDERR_LIMIT));
    let pid = wait_marker(&marker, &lock).await?;
    let cancelled = std::time::Instant::now();
    tx.send(true)?;
    assert_eq!(task.await?, Err(Error::Cancelled));
    assert_released(pid, &lock).await?;
    eprintln!(
        "SEARCH_CHILD pid={pid} cancel_to_reaped_us={}",
        cancelled.elapsed().as_micros()
    );
    let _ = std::fs::remove_file(marker);
    let _ = std::fs::remove_file(lock);
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::print_stderr,
    reason = "The process-lifecycle regression retains PID and cleanup timing evidence"
)]
async fn deadline_kills_waits_and_releases_child_resources() -> anyhow::Result<()> {
    let marker = marker_path();
    let lock = marker.with_extension("lock");
    let child = fixture("wait", Some(&marker))?;
    let pid = wait_marker(&marker, &lock).await?;
    let (_tx, rx) = watch::channel(false);
    let started = std::time::Instant::now();
    assert_eq!(
        collect(
            child,
            rx,
            Duration::from_millis(20),
            STDOUT_LIMIT,
            STDERR_LIMIT
        )
        .await,
        Err(Error::Timeout)
    );
    assert_released(pid, &lock).await?;
    eprintln!(
        "SEARCH_CHILD pid={pid} deadline_us=20000 collection_to_reaped_us={}",
        started.elapsed().as_micros()
    );
    let _ = std::fs::remove_file(marker);
    let _ = std::fs::remove_file(lock);
    Ok(())
}

#[tokio::test]
async fn abrupt_future_drop_uses_kill_on_drop() -> anyhow::Result<()> {
    let marker = marker_path();
    let lock = marker.with_extension("lock");
    let child = fixture("wait", Some(&marker))?;
    let (_tx, rx) = watch::channel(false);
    let task = tokio::spawn(collect(child, rx, TIMEOUT, STDOUT_LIMIT, STDERR_LIMIT));
    let pid = wait_marker(&marker, &lock).await?;
    task.abort();
    let _ = task.await;
    let _ = pid;
    assert_lock_released(&lock).await?;
    let _ = std::fs::remove_file(marker);
    let _ = std::fs::remove_file(lock);
    Ok(())
}

#[test]
fn final_search_argument_is_single_and_follows_option_terminator() {
    for required in [
        "--ignore-config",
        "--no-plugin-dirs",
        "--flat-playlist",
        "--dump-single-json",
        "--playlist-end",
        "5",
        "--skip-download",
    ] {
        assert!(args().contains(&required));
    }
    assert!(!args().contains(&"--no-playlist"));
    assert!(
        !args()
            .iter()
            .any(|arg| arg.contains("runtime") || arg.contains("cookies"))
    );
    let input = "--simulate; $(touch nope)";
    let mut values = args().map(str::to_owned).to_vec();
    values.push(format!("ytsearch5:{input}"));
    assert_eq!(
        values.iter().position(|value| value == "--"),
        Some(values.len() - 2)
    );
    assert_eq!(
        values.last().map(String::as_str),
        Some("ytsearch5:--simulate; $(touch nope)")
    );
    assert_eq!(values.len(), args().len() + 1);
}

#[tokio::test(start_paused = true)]
async fn acquisition_has_a_five_second_bound_and_observes_cancel() -> anyhow::Result<()> {
    let (_tx, mut cancel) = watch::channel(false);
    assert!(matches!(
        acquire_bounded(
            &mut cancel,
            ACQUIRE_LIMIT,
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await,
        Err(Error::MissingComponents)
    ));

    let (tx, mut cancel) = watch::channel(false);
    let acquire = tokio::spawn(async move {
        acquire_bounded(
            &mut cancel,
            ACQUIRE_LIMIT,
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await
    });
    tx.send(true).context("signal acquisition cancellation")?;
    assert_eq!(
        acquire.await.context("join acquisition task")?,
        Err(Error::Cancelled)
    );
    Ok(())
}

#[tokio::test]
async fn timed_out_component_acquisition_releases_the_shared_execution_guard() -> anyhow::Result<()>
{
    let execution = crate::media::components::EXECUTIONS.write().await;
    let (_tx, mut cancel) = watch::channel(false);
    assert!(matches!(
        acquire_bounded(
            &mut cancel,
            Duration::from_millis(20),
            crate::media::components::execution::acquire(),
        )
        .await,
        Err(Error::MissingComponents)
    ));
    drop(execution);
    let acquired = timeout(
        Duration::from_secs(1),
        crate::media::components::execution::acquire(),
    )
    .await
    .context("reacquire execution guard after acquisition timeout")??;
    drop(acquired);
    Ok(())
}

#[tokio::test]
async fn cancelled_component_acquisition_releases_the_shared_execution_guard() -> anyhow::Result<()>
{
    let execution = crate::media::components::EXECUTIONS.write().await;
    let (tx, mut cancel) = watch::channel(false);
    let acquire = tokio::spawn(async move {
        acquire_bounded(
            &mut cancel,
            ACQUIRE_LIMIT,
            crate::media::components::execution::acquire(),
        )
        .await
        .map(|_| ())
    });
    tx.send(true).context("signal acquisition cancellation")?;
    assert_eq!(
        acquire.await.context("join acquisition task")?,
        Err(Error::Cancelled)
    );
    drop(execution);
    let acquired = timeout(
        Duration::from_secs(1),
        crate::media::components::execution::acquire(),
    )
    .await
    .context("reacquire execution guard after cancellation")??;
    drop(acquired);
    Ok(())
}
