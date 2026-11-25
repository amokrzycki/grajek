use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use ed25519_dalek::{Signature, VerifyingKey};

use super::{Pins, version_parts};

pub(crate) mod fetch;

const MANIFEST_FILE: &str = "component-manifest.json";
const SIGNATURE_FILE: &str = "component-manifest.json.sig";

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
    parse_key(include_str!("../../../packaging/components.pub"))
}

fn parse_key(hex: &str) -> anyhow::Result<VerifyingKey> {
    let hex = hex.trim();
    anyhow::ensure!(
        hex.len() == 64,
        "The component public key must be 64 hex characters."
    );
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Accepted {
    Stored,
    Current,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckError {
    Unavailable,
    Rejected(Rejection),
}

pub(crate) fn accept(
    dir: &Path,
    manifest: &[u8],
    signature: &[u8],
    key: &VerifyingKey,
) -> Result<Accepted, CheckError> {
    let embedded = super::embedded_pins().map_err(|_| CheckError::Unavailable)?;
    let stored = stored_pins(dir, key, 0).map_or(0, |pins| pins.serial);
    let floor = embedded.serial.max(stored);
    match verify(manifest, signature, key, floor) {
        Ok(_) => store_manifest(dir, manifest, signature)
            .map_err(|_| CheckError::Unavailable)
            .map(|()| Accepted::Stored),
        Err(Rejection::NotNewer) => Ok(Accepted::Current),
        Err(rejection) => Err(CheckError::Rejected(rejection)),
    }
}

pub(crate) fn store_manifest(dir: &Path, manifest: &[u8], signature: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(MANIFEST_FILE), manifest)?;
    write_atomic(&dir.join(SIGNATURE_FILE), signature)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::File::create(&temp)?;
        std::io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(super) fn stored_pins(dir: &Path, key: &VerifyingKey, floor: u64) -> Option<Pins> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let warn_once = |what: &str| {
        if !WARNED.swap(true, Ordering::Relaxed) {
            tracing::warn!("Ignoring stored component manifest: {what}");
        }
    };
    let read = |name: &str| match fs::read(dir.join(name)) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            warn_once(&format!("unreadable {name} ({error})"));
            None
        }
    };
    let manifest = read(MANIFEST_FILE)?;
    let signature = read(SIGNATURE_FILE)?;
    match verify(&manifest, &signature, key, floor) {
        Ok(pins) => Some(pins),
        Err(rejection) => {
            warn_once(&format!("verification rejected it ({rejection:?})"));
            None
        }
    }
}

#[cfg(test)]
mod tests;
