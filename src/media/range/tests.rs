use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

struct Server {
    url: String,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(bytes: Vec<u8>, fault: &'static str) -> Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/audio", listener.local_addr()?);
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8_lossy(&request).to_lowercase();
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="));
            let Some((start, end)) = range.and_then(|range| range.split_once('-')) else {
                return;
            };
            let Ok(start) = start.parse::<usize>() else {
                return;
            };
            let Ok(end) = end.parse::<usize>() else {
                return;
            };
            if fault == "later" && start != 0 {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                continue;
            }
            let end = end.min(bytes.len() - 1);
            let first = if fault == "offset" { start + 1 } else { start };
            let status = if fault == "ignored" {
                "200 OK"
            } else {
                "206 Partial Content"
            };
            let body = &bytes[start..=end];
            let total = if fault == "length" {
                bytes.len() + 1
            } else {
                bytes.len()
            };
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Range: bytes {first}-{end}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if socket.write_all(header.as_bytes()).await.is_err() {
                continue;
            }
            let body = if fault == "truncated" {
                &body[..body.len() / 2]
            } else {
                body
            };
            let _ = socket.write_all(body).await;
        }
    });
    Ok(Server { url, task })
}

fn data() -> Vec<u8> {
    (0..800_000_u32).flat_map(u32::to_le_bytes).collect()
}

#[tokio::test]
async fn ranges_support_both_directions_eof_and_overflow_safe_offsets() -> Result<()> {
    let bytes = data();
    let server = server(bytes.clone(), "good").await?;
    let mut source = RangeSource::open(Client::new(), server.url.clone(), HeaderMap::new()).await?;
    assert!(source.bytes.len() <= usize::try_from(WINDOW)?);
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut actual = [0; 80];
        for offset in [0, 900_000, 100, 1_500_000, 65] {
            source.seek(SeekFrom::Start(offset))?;
            source.read_exact(&mut actual)?;
            let offset = usize::try_from(offset)?;
            assert_eq!(actual, bytes[offset..offset + actual.len()]);
            assert!(source.bytes.len() <= usize::try_from(WINDOW)?);
        }
        assert!(source.seek(SeekFrom::Start(u64::MAX)).is_err());
        assert!(source.seek(SeekFrom::Current(i64::MIN)).is_err());
        assert!(source.seek(SeekFrom::End(i64::MAX)).is_err());
        source.seek(SeekFrom::End(0))?;
        assert_eq!(source.read(&mut actual)?, 0);
        source.seek(SeekFrom::End(-1))?;
        assert_eq!(source.read(&mut actual)?, 1);
        source.control.stop();
        let error = source
            .read(&mut actual)
            .err()
            .context("cancelled source still readable")?;
        assert_ne!(
            error.kind(),
            io::ErrorKind::Interrupted,
            "read_exact retries Interrupted forever"
        );
        assert!(source.read_exact(&mut actual).is_err());
        assert!(
            !source.control.failed(),
            "cancellation is not a network failure"
        );
        Ok(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn later_range_failure_is_signalled_after_reading_or_seeking() -> Result<()> {
    let server = server(data(), "later").await?;
    for seek in [false, true] {
        let mut source =
            RangeSource::open(Client::new(), server.url.clone(), HeaderMap::new()).await?;
        let control = source.control.clone();
        assert!(!control.failed());
        tokio::task::spawn_blocking(move || -> Result<()> {
            if seek {
                source.seek(SeekFrom::Start(WINDOW + 1))?;
            } else {
                let mut first = vec![0; usize::try_from(WINDOW)?];
                source.read_exact(&mut first)?;
            }
            assert!(source.read_exact(&mut [0]).is_err());
            Ok(())
        })
        .await??;
        assert!(
            control.failed(),
            "packet readers must retain the HTTP failure"
        );
    }
    Ok(())
}

#[tokio::test]
async fn ignored_misaligned_truncated_or_changed_ranges_are_rejected() -> Result<()> {
    for fault in ["ignored", "offset", "truncated"] {
        let server = server(data(), fault).await?;
        assert!(
            RangeSource::open(Client::new(), server.url.clone(), HeaderMap::new())
                .await
                .is_err(),
            "{fault}"
        );
    }
    let server = server(data(), "length").await?;
    assert!(
        fetch(
            &Client::new(),
            &server.url,
            &HeaderMap::new(),
            0,
            Some(3_200_000)
        )
        .await
        .is_err()
    );
    Ok(())
}

#[test]
fn cancellation_read_budget_and_deadline_bound_unindexed_seeks() {
    let control = SourceControl::default();
    control.limit();
    assert!(control.consume(4 * 1024 * 1024).is_ok());
    assert!(control.consume(1).is_err());
    control.clear_limit();
    assert!(control.consume(1).is_ok());
    control
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .budget = Some((Instant::now(), usize::MAX));
    assert!(control.consume(0).is_err());
    control.clear_limit();
    control.stop();
    assert!(control.consume(0).is_err());
}
