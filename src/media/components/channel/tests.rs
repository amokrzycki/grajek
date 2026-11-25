use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

use super::{MAX_MANIFEST, Rejection, embedded_key, verify};

fn keypair() -> (SigningKey, VerifyingKey) {
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let verifying = signing.verifying_key();
    (signing, verifying)
}

fn manifest() -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../../packaging/components.json"
    ))?)
}

fn signed(
    signing: &SigningKey,
    value: &mut serde_json::Value,
    serial: u64,
    min_app: &str,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    value["serial"] = serial.into();
    value["min_app"] = min_app.into();
    let bytes = serde_json::to_vec(value)?;
    let signature = signing.sign(&bytes);
    Ok((bytes, signature.to_bytes().to_vec()))
}

#[test]
fn valid_newer_manifest_is_accepted() -> anyhow::Result<()> {
    let (signing, verifying) = keypair();
    let mut value = manifest()?;
    let (bytes, signature) = signed(&signing, &mut value, 2, "0.1.0")?;
    let accepted = verify(&bytes, &signature, &verifying, 1);
    assert!(matches!(accepted, Ok(pins) if pins.serial == 2));
    Ok(())
}

#[test]
fn equal_serial_is_up_to_date() -> anyhow::Result<()> {
    let (signing, verifying) = keypair();
    let mut value = manifest()?;
    let (bytes, signature) = signed(&signing, &mut value, 1, "0.1.0")?;
    assert!(matches!(
        verify(&bytes, &signature, &verifying, 1),
        Err(Rejection::NotNewer)
    ));
    Ok(())
}

#[test]
fn tampered_byte_fails_signature() -> anyhow::Result<()> {
    let (signing, verifying) = keypair();
    let mut value = manifest()?;
    let (mut bytes, signature) = signed(&signing, &mut value, 2, "0.1.0")?;
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert!(matches!(
        verify(&bytes, &signature, &verifying, 1),
        Err(Rejection::BadSignature)
    ));
    Ok(())
}

#[test]
fn oversized_manifest_is_rejected_before_parsing() -> anyhow::Result<()> {
    let verifying = embedded_key()?;
    let bytes = vec![b'x'; MAX_MANIFEST + 1];
    assert!(matches!(
        verify(&bytes, &[], &verifying, 0),
        Err(Rejection::TooLarge)
    ));
    Ok(())
}

#[test]
fn untrusted_artifact_url_is_rejected() -> anyhow::Result<()> {
    let urls = [
        "https://example.com/yt-dlp_linux.zip",
        "http://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp_linux.zip",
    ];
    for url in urls {
        let (signing, verifying) = keypair();
        let mut value = manifest()?;
        #[cfg(target_os = "linux")]
        {
            value["yt-dlp"]["linux"]["url"] = url.into();
        }
        #[cfg(target_os = "windows")]
        {
            value["yt-dlp"]["windows"]["url"] = url.into();
        }
        let (bytes, signature) = signed(&signing, &mut value, 2, "0.1.0")?;
        assert!(
            matches!(
                verify(&bytes, &signature, &verifying, 1),
                Err(Rejection::UntrustedUrl)
            ),
            "url: {url}"
        );
    }
    Ok(())
}

#[test]
fn newer_min_app_is_rejected() -> anyhow::Result<()> {
    for min_app in ["99.0.0", "abc"] {
        let (signing, verifying) = keypair();
        let mut value = manifest()?;
        let (bytes, signature) = signed(&signing, &mut value, 2, min_app)?;
        assert!(
            matches!(
                verify(&bytes, &signature, &verifying, 1),
                Err(Rejection::AppTooOld)
            ),
            "min_app: {min_app}"
        );
    }
    Ok(())
}

#[test]
fn embedded_key_parses() -> anyhow::Result<()> {
    let key = embedded_key()?;
    assert_eq!(key.to_bytes().len(), 32);
    Ok(())
}

#[test]
fn short_signature_is_rejected() -> anyhow::Result<()> {
    let (signing, verifying) = keypair();
    let mut value = manifest()?;
    let (bytes, signature) = signed(&signing, &mut value, 2, "0.1.0")?;
    assert!(matches!(
        verify(&bytes, &signature[..63], &verifying, 1),
        Err(Rejection::BadSignature)
    ));
    Ok(())
}
