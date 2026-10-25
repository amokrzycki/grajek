use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

const DEADLINE_ERROR: &str =
    "Stream availability wait exceeds the remaining audio preparation deadline. Try again.";

fn selected_timestamp(metadata: &Value) -> Option<&Value> {
    metadata.get("available_at").or_else(|| {
        let url = metadata.get("url")?.as_str()?;
        metadata
            .get("requested_formats")?
            .as_array()?
            .iter()
            .find_map(|format| {
                (format.get("url").and_then(Value::as_str) == Some(url))
                    .then(|| format.get("available_at"))
                    .flatten()
            })
    })
}

fn delay(timestamp: Option<&Value>, now: Duration, remaining: Duration) -> Result<Duration> {
    let Some(seconds) = timestamp
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
    else {
        return Ok(Duration::ZERO);
    };
    let available_at = Duration::try_from_secs_f64(seconds).context(DEADLINE_ERROR)?;
    // yt-dlp subtracts int(time.time()); preserve its whole-second safety margin.
    let delay = available_at.saturating_sub(Duration::from_secs(now.as_secs()));
    ensure!(
        delay.is_zero() || delay < remaining,
        "Stream availability wait ({:.2}s) equals or exceeds the remaining audio preparation deadline ({:.2}s). Try again.",
        delay.as_secs_f64(),
        remaining.as_secs_f64()
    );
    Ok(delay)
}

pub(super) async fn wait_for_stream(metadata: &Value, deadline: Instant) -> Result<()> {
    wait(
        selected_timestamp(metadata),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System clock is before the Unix epoch.")?,
        deadline,
    )
    .await
}

async fn wait(timestamp: Option<&Value>, now: Duration, deadline: Instant) -> Result<()> {
    let delay = delay(
        timestamp,
        now,
        deadline.saturating_duration_since(Instant::now()),
    )?;
    if !delay.is_zero() {
        tracing::info!(
            stage = "stream_availability",
            delay_seconds = delay.as_secs_f64(),
            "Waiting for yt-dlp stream availability"
        );
        tokio::time::sleep(delay).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_selected_stream_controls_availability() {
        let metadata = json!({
            "url": "audio", "available_at": 105,
            "formats": [{"url": "other", "available_at": 999}],
        });
        assert_eq!(selected_timestamp(&metadata), Some(&json!(105)));
        let metadata = json!({
            "url": "audio",
            "requested_formats": [
                {"url": "video", "available_at": 999},
                {"url": "audio", "available_at": 105.5},
            ],
        });
        assert_eq!(selected_timestamp(&metadata), Some(&json!(105.5)));
        assert_eq!(
            selected_timestamp(
                &json!({"url": "other", "requested_formats": metadata["requested_formats"]})
            ),
            None
        );
        assert_eq!(selected_timestamp(&json!({"url": "audio"})), None);
    }

    #[test]
    fn availability_delay_handles_optional_untrusted_timestamps() -> anyhow::Result<()> {
        let now = Duration::from_millis(100_750);
        let remaining = Duration::from_secs(10);
        for value in [
            json!(null),
            json!("105"),
            json!(true),
            json!([]),
            json!({}),
            json!(-1),
            json!(0),
            json!(99),
            json!(100),
        ] {
            assert_eq!(
                delay(Some(&value), now, remaining)?,
                Duration::ZERO,
                "{value}"
            );
        }
        assert_eq!(delay(None, now, remaining)?, Duration::ZERO);
        // Match yt-dlp's subtraction of int(time.time()), including fractional ad skip times.
        assert_eq!(
            delay(Some(&json!(105)), now, remaining)?,
            Duration::from_secs(5)
        );
        assert_eq!(
            delay(Some(&json!(105.5)), now, remaining)?,
            Duration::from_millis(5500)
        );
        for value in [json!(110), json!(111), json!(u64::MAX), json!(1e300)] {
            let error = delay(Some(&value), now, remaining)
                .err()
                .context("accepted impossible wait")?;
            assert!(error.to_string().contains("preparation deadline"));
        }
        assert!(delay(Some(&json!(101)), now, Duration::ZERO).is_err());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn normal_streams_return_without_yielding_or_advancing_time() -> Result<()> {
        let start = Instant::now();
        for timestamp in [None, Some(json!(99)), Some(json!(100))] {
            tokio::select! {
                biased;
                result = wait(timestamp.as_ref(), Duration::from_secs(100), start + Duration::from_secs(60)) => result?,
                () = std::future::ready(()) => anyhow::bail!("unnecessary availability wait"),
            }
        }
        assert_eq!(Instant::now(), start);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn future_stream_waits_and_can_be_cancelled_before_access() -> Result<()> {
        let start = Instant::now();
        let timestamp = json!(105);
        let waiting = wait(
            Some(&timestamp),
            Duration::from_secs(100),
            start + Duration::from_secs(60),
        );
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            result = &mut waiting => anyhow::bail!("opened future stream: {result:?}"),
            () = std::future::ready(()) => {},
        }
        tokio::time::advance(Duration::from_secs(4)).await;
        tokio::select! {
            biased;
            result = &mut waiting => anyhow::bail!("opened future stream: {result:?}"),
            () = std::future::ready(()) => {},
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        waiting.await?;
        assert_eq!(Instant::now() - start, Duration::from_secs(5));

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (access_tx, mut access_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = started_tx.send(());
            wait(
                Some(&json!(105)),
                Duration::from_secs(100),
                Instant::now() + Duration::from_secs(60),
            )
            .await?;
            let _ = access_tx.send(());
            Ok::<_, anyhow::Error>(())
        });
        started_rx.await?;
        task.abort();
        assert!(task.await.is_err_and(|error| error.is_cancelled()));
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(matches!(
            access_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn extraction_time_counts_against_the_preparation_deadline() -> Result<()> {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(60);
        tokio::time::advance(Duration::from_secs(58)).await;
        let error = wait(Some(&json!(105)), Duration::from_secs(100), deadline)
            .await
            .err()
            .context("deadline ignored")?;
        assert!(error.to_string().contains("preparation deadline"));
        assert_eq!(Instant::now() - start, Duration::from_secs(58));
        Ok(())
    }
}
