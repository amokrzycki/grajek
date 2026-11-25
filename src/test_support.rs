//! Fixtures shared by in-crate tests: a private scratch directory and canned domain data.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    favorites::FavoriteEntry,
    history::HistoryEntry,
    search::{ResultItem, Results},
};

/// An empty directory unique to this process and call, removed on drop.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new() -> io::Result<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "grajek-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn favorite(url: &str, title: Option<&str>) -> FavoriteEntry {
    FavoriteEntry {
        url: url.into(),
        title: title.map(str::to_owned),
        channel: None,
        duration_secs: None,
        live: false,
    }
}

pub(crate) fn history_entry(id: &str) -> HistoryEntry {
    HistoryEntry {
        url: format!("https://www.youtube.com/watch?v={id:0>11}"),
        title: Some(format!("Song {id}")),
        duration_secs: Some(225),
    }
}

pub(crate) fn results(url: &str) -> Results {
    Results {
        items: vec![ResultItem {
            url: url.into(),
            title: Some("Factual title".into()),
            channel: None,
            duration: None,
            live: false,
        }],
        rejected: 0,
    }
}

pub(crate) fn component_keypair() -> (ed25519_dalek::SigningKey, ed25519_dalek::VerifyingKey) {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let verifying = signing.verifying_key();
    (signing, verifying)
}

pub(crate) fn component_manifest() -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(include_str!(
        "../packaging/components.json"
    ))?)
}

pub(crate) fn signed_manifest(
    signing: &ed25519_dalek::SigningKey,
    value: &mut serde_json::Value,
    serial: u64,
    min_app: &str,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    use ed25519_dalek::Signer as _;
    value["serial"] = serial.into();
    value["min_app"] = min_app.into();
    let bytes = serde_json::to_vec(value)?;
    let signature = signing.sign(&bytes);
    Ok((bytes, signature.to_bytes().to_vec()))
}
