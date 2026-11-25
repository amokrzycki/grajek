use super::{CheckError, Rejection, check, fetch};
use crate::test_support::{Scratch, server};

#[tokio::test]
async fn fetch_returns_manifest_and_signature() -> anyhow::Result<()> {
    let manifest = b"{\"serial\":2}".to_vec();
    let signature = vec![7u8; 64];
    let server = server(vec![
        ("/components.json", 200, manifest.clone()),
        ("/components.json.sig", 200, signature.clone()),
    ])
    .await?;
    let client = reqwest::Client::new();
    let (got_manifest, got_signature) = match fetch(&client, &server.base).await {
        Ok(parts) => parts,
        Err(error) => anyhow::bail!("fetch failed: {error:?}"),
    };
    assert_eq!(got_manifest, manifest);
    assert_eq!(got_signature, signature);
    Ok(())
}

#[tokio::test]
async fn missing_manifest_is_unavailable() -> anyhow::Result<()> {
    let server = server(vec![]).await?;
    let client = reqwest::Client::new();
    assert!(matches!(
        fetch(&client, &server.base).await,
        Err(CheckError::Unavailable)
    ));
    Ok(())
}

#[tokio::test]
async fn oversized_manifest_is_rejected_while_streaming() -> anyhow::Result<()> {
    let server = server(vec![
        ("/components.json", 200, vec![b'x'; 70_000]),
        ("/components.json.sig", 200, vec![7u8; 64]),
    ])
    .await?;
    let client = reqwest::Client::new();
    assert!(matches!(
        fetch(&client, &server.base).await,
        Err(CheckError::Rejected(Rejection::TooLarge))
    ));
    Ok(())
}

#[tokio::test]
async fn long_signature_is_rejected() -> anyhow::Result<()> {
    let server = server(vec![
        ("/components.json", 200, b"{}".to_vec()),
        ("/components.json.sig", 200, vec![7u8; 65]),
    ])
    .await?;
    let client = reqwest::Client::new();
    assert!(matches!(
        fetch(&client, &server.base).await,
        Err(CheckError::Rejected(Rejection::TooLarge))
    ));
    Ok(())
}

#[tokio::test]
async fn check_reports_unavailable_when_release_is_missing() -> anyhow::Result<()> {
    let server = server(vec![]).await?;
    let dir = Scratch::new()?;
    let client = reqwest::Client::new();
    assert_eq!(
        check(&client, &server.base, dir.path()).await,
        Err(CheckError::Unavailable)
    );
    assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
    Ok(())
}
