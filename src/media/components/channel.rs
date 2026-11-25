use ed25519_dalek::{Signature, VerifyingKey};

use super::{Pins, version_parts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    TooLarge,
    BadSignature,
    Malformed,
    NotNewer,
    AppTooOld,
    UntrustedUrl,
}

const MAX_MANIFEST: usize = 64 * 1024;

const TRUSTED_PREFIXES: [&str; 3] = [
    "https://github.com/yt-dlp/yt-dlp/releases/download/",
    "https://github.com/denoland/deno/releases/download/",
    "https://dl.deno.land/release/",
];

pub(crate) fn embedded_key() -> anyhow::Result<VerifyingKey> {
    let hex = include_str!("../../../packaging/components.pub").trim();
    anyhow::ensure!(hex.len() == 64, "embedded key must be 64 hex chars");
    let (chunks, _) = hex.as_bytes().as_chunks::<2>();
    let mut bytes = [0u8; 32];
    for (slot, chunk) in bytes.iter_mut().zip(chunks) {
        *slot = u8::from_str_radix(std::str::from_utf8(chunk)?, 16)?;
    }
    Ok(VerifyingKey::from_bytes(&bytes)?)
}

pub(crate) fn verify(
    manifest: &[u8],
    signature: &[u8],
    key: &VerifyingKey,
    floor: u64,
) -> Result<Pins, Rejection> {
    if manifest.len() > MAX_MANIFEST {
        return Err(Rejection::TooLarge);
    }
    let signature = Signature::try_from(signature).map_err(|_| Rejection::BadSignature)?;
    key.verify_strict(manifest, &signature)
        .map_err(|_| Rejection::BadSignature)?;
    let pins: Pins = serde_json::from_slice(manifest).map_err(|_| Rejection::Malformed)?;
    for pin in [&pins.ytdlp, &pins.deno] {
        if !TRUSTED_PREFIXES
            .iter()
            .any(|prefix| pin.artifact().url.starts_with(prefix))
        {
            return Err(Rejection::UntrustedUrl);
        }
    }
    let current = version_parts(env!("CARGO_PKG_VERSION"));
    let required = version_parts(&pins.min_app);
    if required.is_none() || required > current {
        return Err(Rejection::AppTooOld);
    }
    if pins.serial <= floor {
        return Err(Rejection::NotNewer);
    }
    Ok(pins)
}

#[cfg(test)]
mod tests;
