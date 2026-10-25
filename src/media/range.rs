//! Bounded HTTP byte ranges for Symphonia's indexed container seeking.
use anyhow::{Context, Result, ensure};
use reqwest::{
    Client, StatusCode,
    header::{CONTENT_RANGE, HeaderMap, RANGE},
};
use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use symphonia::core::{
    formats::{Cue, FormatOptions, FormatReader, Packet, SeekMode, SeekTo, SeekedTo, Track},
    io::{MediaSource, MediaSourceStream},
    meta::Metadata,
};

const WINDOW: u64 = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub(crate) struct SourceControl(Arc<Mutex<SourceState>>);

#[derive(Default)]
struct SourceState {
    stopped: bool,
    failed: bool,
    streaming_probe: bool,
    budget: Option<(Instant, usize)>,
}

impl SourceControl {
    pub(crate) fn failed(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).failed
    }
    pub(crate) fn record_failure(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).failed = true;
    }
    pub(super) fn enable_seeking(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .streaming_probe = false;
    }
    pub(crate) fn stop(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopped = true;
    }
    pub(super) fn limit(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).budget =
            Some((Instant::now() + Duration::from_secs(8), 4 * 1024 * 1024));
    }
    pub(super) fn clear_limit(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).budget = None;
    }
    fn consume(&self, size: usize) -> io::Result<()> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.stopped {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Track cancelled",
            ));
        }
        if let Some((deadline, remaining)) = &mut state.budget {
            if Instant::now() >= *deadline || size > *remaining {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Stream seek exceeded its time/read budget",
                ));
            }
            *remaining -= size;
        }
        drop(state);
        Ok(())
    }
}

/// A container without an index may scan packets. Bound that work even after the
/// controller stops waiting for a Songbird callback (dropping it cannot cancel it).
pub(super) struct SeekReader {
    pub(super) inner: Box<dyn FormatReader>,
    pub(super) control: SourceControl,
}

impl FormatReader for SeekReader {
    fn try_new(_: MediaSourceStream, _: &FormatOptions) -> symphonia::core::errors::Result<Self> {
        Err(symphonia::core::errors::Error::Unsupported(
            "SeekReader wraps an already parsed format",
        ))
    }
    fn cues(&self) -> &[Cue] {
        self.inner.cues()
    }
    fn metadata(&mut self) -> Metadata<'_> {
        self.inner.metadata()
    }
    fn tracks(&self) -> &[Track] {
        self.inner.tracks()
    }
    fn default_track(&self) -> Option<&Track> {
        self.inner.default_track()
    }
    fn next_packet(&mut self) -> symphonia::core::errors::Result<Packet> {
        self.inner.next_packet()
    }
    fn into_inner(self: Box<Self>) -> MediaSourceStream {
        self.inner.into_inner()
    }
    fn seek(&mut self, mode: SeekMode, to: SeekTo) -> symphonia::core::errors::Result<SeekedTo> {
        self.control.limit();
        let result = self.inner.seek(mode, to);
        self.control.clear_limit();
        result
    }
}

pub(super) struct RangeSource {
    pub(super) control: SourceControl,
    client: Client,
    url: String,
    headers: HeaderMap,
    len: u64,
    position: u64,
    start: u64,
    bytes: Vec<u8>,
    runtime: tokio::runtime::Handle,
}

impl RangeSource {
    pub(super) async fn probe(client: Client, url: String, headers: HeaderMap) -> Option<Self> {
        match Self::open(client, url, headers).await {
            Ok(source) => Some(source),
            Err(error) => {
                tracing::warn!(
                    stage = "http_range_probe",
                    error = %format!("{error:#}"),
                    "HTTP range probe failed; trying Songbird streaming without seeking"
                );
                None
            }
        }
    }

    pub(super) async fn open(client: Client, url: String, mut headers: HeaderMap) -> Result<Self> {
        headers.remove(RANGE);
        let (bytes, len) = fetch(&client, &url, &headers, 0, None).await?;
        // Symphonia's seekable MP4 probe reads every top-level atom, including mdat.
        // Probe streamable MP4 only up to its index/data, then enable byte seeking.
        let control = SourceControl::default();
        control
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .streaming_probe = bytes.get(4..8) == Some(b"ftyp");
        Ok(Self {
            control,
            client,
            url,
            headers,
            len,
            position: 0,
            start: 0,
            bytes,
            runtime: tokio::runtime::Handle::current(),
        })
    }
}

async fn fetch(
    client: &Client,
    url: &str,
    headers: &HeaderMap,
    start: u64,
    len: Option<u64>,
) -> Result<(Vec<u8>, u64)> {
    let end = start
        .saturating_add(WINDOW - 1)
        .min(len.unwrap_or(u64::MAX).saturating_sub(1));
    let mut response = client
        .get(url)
        .headers(headers.clone())
        .header(RANGE, format!("bytes={start}-{end}"))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(reqwest::Error::without_url)
        .context("HTTP range request failed")?;
    let status = response.status();
    ensure!(status.is_success(), "HTTP range request rejected: {status}");
    ensure!(
        status == StatusCode::PARTIAL_CONTENT,
        "Stream does not honor HTTP byte ranges (HTTP {status})"
    );
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .context("Missing HTTP Content-Range")?;
    let (interval, total) = range
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .context("Invalid Content-Range")?;
    let (first, last) = interval.split_once('-').context("Invalid range interval")?;
    let total: u64 = total.parse()?;
    let last: u64 = last.parse()?;
    ensure!(
        first.parse::<u64>()? == start
            && last >= start
            && last <= end
            && last < total
            && len.is_none_or(|len| len == total),
        "Mismatched HTTP byte range"
    );
    let expected = usize::try_from(last - start + 1)?;
    let mut bytes = Vec::with_capacity(expected);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)
        .context("HTTP range response read failed")?
    {
        ensure!(
            chunk.len() <= expected - bytes.len(),
            "Oversized HTTP range response"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(bytes.len() == expected, "Truncated HTTP range response");
    Ok((bytes, total))
}

impl Read for RangeSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.control.consume(0)?;
        if out.is_empty() || self.position == self.len {
            return Ok(0);
        }
        if self.position < self.start || self.position - self.start >= self.bytes.len() as u64 {
            // Symphonia's synchronous I/O runs outside the controller and GPUI.
            let (bytes, _) = self
                .runtime
                .block_on(fetch(
                    &self.client,
                    &self.url,
                    &self.headers,
                    self.position,
                    Some(self.len),
                ))
                .map_err(|error| {
                    self.control.record_failure();
                    io::Error::other(error.to_string())
                })?;
            self.start = self.position;
            self.bytes = bytes;
        }
        let offset = usize::try_from(self.position - self.start).map_err(io::Error::other)?;
        let n = out.len().min(self.bytes.len() - offset);
        self.control.consume(n)?;
        out[..n].copy_from_slice(&self.bytes[offset..offset + n]);
        self.position += n as u64;
        Ok(n)
    }
}

impl Seek for RangeSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
        }
        .filter(|pos| *pos <= self.len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Seek outside stream"))?;
        self.position = position;
        Ok(position)
    }
}

impl MediaSource for RangeSource {
    fn is_seekable(&self) -> bool {
        !self
            .control
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .streaming_probe
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
#[path = "range/tests.rs"]
mod tests;
