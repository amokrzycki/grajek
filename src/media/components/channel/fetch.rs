use std::{path::Path, time::Duration};

use super::{Accepted, CheckError, MAX_MANIFEST, Rejection, accept, embedded_key};

pub(crate) const MANIFEST_BASE: &str =
    "https://github.com/amokrzycki/grajek/releases/download/components";

const MAX_SIGNATURE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FetchError {
    Unavailable,
    TooLarge,
}

pub(crate) fn client() -> reqwest::Result<reqwest::Client> {
    super::super::download::http_builder()
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .build()
}

pub(crate) async fn fetch(
    client: &reqwest::Client,
    base: &str,
) -> Result<(Vec<u8>, Vec<u8>), FetchError> {
    let manifest = body(client, &format!("{base}/components.json"), MAX_MANIFEST).await?;
    let signature = body(
        client,
        &format!("{base}/components.json.sig"),
        MAX_SIGNATURE,
    )
    .await?;
    Ok((manifest, signature))
}

async fn body(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, FetchError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| FetchError::Unavailable)?
        .error_for_status()
        .map_err(|_| FetchError::Unavailable)?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| FetchError::Unavailable)?
    {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(FetchError::TooLarge);
        }
    }
    Ok(body)
}

pub(crate) async fn check(
    client: &reqwest::Client,
    base: &str,
    dir: &Path,
) -> Result<Accepted, CheckError> {
    let (manifest, signature) = match fetch(client, base).await {
        Ok(parts) => parts,
        Err(FetchError::Unavailable) => return Err(CheckError::Unavailable),
        Err(FetchError::TooLarge) => return Err(CheckError::Rejected(Rejection::TooLarge)),
    };
    let key = embedded_key().map_err(|_| CheckError::Unavailable)?;
    accept(dir, &manifest, &signature, &key)
}

#[cfg(test)]
mod tests;
