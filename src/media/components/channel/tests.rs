use ed25519_dalek::Signer as _;

use super::{
    Accepted, CheckError, MANIFEST_FILE, MAX_MANIFEST, Rejection, SIGNATURE_FILE, accept,
    embedded_key, parse_key, store_manifest, stored_pins, verify,
};
use crate::test_support::{
    Scratch, component_keypair, component_manifest, embedded_serial, signed_manifest,
};

#[test]
fn valid_newer_manifest_is_accepted() -> anyhow::Result<()> {
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    let accepted = verify(&bytes, &signature, &verifying, base);
    assert!(matches!(accepted, Ok(pins) if pins.serial == base + 1));
    Ok(())
}

#[test]
fn equal_serial_is_up_to_date() -> anyhow::Result<()> {
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base, "0.1.0")?;
    assert!(matches!(
        verify(&bytes, &signature, &verifying, base),
        Err(Rejection::NotNewer)
    ));
    Ok(())
}

#[test]
fn tampered_byte_fails_signature() -> anyhow::Result<()> {
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (mut bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert!(matches!(
        verify(&bytes, &signature, &verifying, base),
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
        let (signing, verifying) = component_keypair();
        let mut value = component_manifest()?;
        let base = embedded_serial()?;
        #[cfg(target_os = "linux")]
        {
            value["yt-dlp"]["linux"]["url"] = url.into();
        }
        #[cfg(target_os = "windows")]
        {
            value["yt-dlp"]["windows"]["url"] = url.into();
        }
        let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
        assert!(
            matches!(
                verify(&bytes, &signature, &verifying, base),
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
        let (signing, verifying) = component_keypair();
        let mut value = component_manifest()?;
        let base = embedded_serial()?;
        let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, min_app)?;
        assert!(
            matches!(
                verify(&bytes, &signature, &verifying, base),
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
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    assert!(matches!(
        verify(&bytes, &signature[..63], &verifying, base),
        Err(Rejection::BadSignature)
    ));
    Ok(())
}

#[test]
fn stored_manifest_with_bad_signature_falls_back_to_embedded() -> anyhow::Result<()> {
    let dir = Scratch::new()?;
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    store_manifest(dir.path(), &bytes, &signature)?;

    let mut flipped = signature.clone();
    flipped[0] ^= 0xFF;
    std::fs::write(dir.path().join(SIGNATURE_FILE), &flipped)?;
    assert!(stored_pins(dir.path(), &verifying, base).is_none());
    assert_eq!(
        super::super::pins_with(Some(dir.path()), &verifying)?.serial,
        base
    );

    std::fs::write(dir.path().join(SIGNATURE_FILE), &signature[..63])?;
    assert!(stored_pins(dir.path(), &verifying, base).is_none());

    let corrupt = b"{not json";
    let corrupt_signature = signing.sign(corrupt).to_bytes().to_vec();
    std::fs::write(dir.path().join(MANIFEST_FILE), corrupt)?;
    std::fs::write(dir.path().join(SIGNATURE_FILE), &corrupt_signature)?;
    assert!(stored_pins(dir.path(), &verifying, base).is_none());
    Ok(())
}

#[test]
fn embedded_pins_win_when_serial_is_higher() -> anyhow::Result<()> {
    let dir = Scratch::new()?;
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) =
        signed_manifest(&signing, &mut value, base.saturating_sub(1), "0.1.0")?;
    store_manifest(dir.path(), &bytes, &signature)?;
    assert!(stored_pins(dir.path(), &verifying, base).is_none());

    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    store_manifest(dir.path(), &bytes, &signature)?;
    assert!(
        matches!(stored_pins(dir.path(), &verifying, base), Some(pins) if pins.serial == base + 1)
    );
    Ok(())
}

#[test]
fn accept_stores_newer_manifest() -> anyhow::Result<()> {
    let dir = Scratch::new()?;
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    assert_eq!(
        accept(dir.path(), &bytes, &signature, &verifying),
        Ok(Accepted::Stored)
    );
    assert!(
        matches!(stored_pins(dir.path(), &verifying, base), Some(pins) if pins.serial == base + 1)
    );
    Ok(())
}

#[test]
fn accept_reports_equal_serial_as_current() -> anyhow::Result<()> {
    let dir = Scratch::new()?;
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    assert_eq!(
        accept(dir.path(), &bytes, &signature, &verifying),
        Ok(Accepted::Stored)
    );
    assert_eq!(
        accept(dir.path(), &bytes, &signature, &verifying),
        Ok(Accepted::Current)
    );
    Ok(())
}

#[test]
fn accept_rejects_bad_signature_without_storing() -> anyhow::Result<()> {
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    let (mut newer, newer_signature) = signed_manifest(&signing, &mut value, base + 2, "0.1.0")?;
    let last = newer.len() - 1;
    newer[last] ^= 0xFF;

    let empty = Scratch::new()?;
    assert_eq!(
        accept(empty.path(), &newer, &newer_signature, &verifying),
        Err(CheckError::Rejected(Rejection::BadSignature))
    );
    assert_eq!(std::fs::read_dir(empty.path())?.count(), 0);

    let dir = Scratch::new()?;
    store_manifest(dir.path(), &bytes, &signature)?;
    assert_eq!(
        accept(dir.path(), &newer, &newer_signature, &verifying),
        Err(CheckError::Rejected(Rejection::BadSignature))
    );
    assert!(
        matches!(stored_pins(dir.path(), &verifying, base), Some(pins) if pins.serial == base + 1)
    );
    Ok(())
}

#[test]
#[ignore = "reads GRAJEK_SIGNED_MANIFEST and GRAJEK_SIGNED_MANIFEST_SIG; see packaging/README.md"]
fn script_signature_matches_rust_verifier() -> anyhow::Result<()> {
    let manifest = std::fs::read(std::env::var("GRAJEK_SIGNED_MANIFEST")?)?;
    let signature = std::fs::read(std::env::var("GRAJEK_SIGNED_MANIFEST_SIG")?)?;
    let key = match std::env::var("GRAJEK_COMPONENTS_PUB") {
        Ok(path) => parse_key(&std::fs::read_to_string(path)?)?,
        Err(_) => embedded_key()?,
    };
    verify(&manifest, &signature, &key, 0).map_err(|rejection| anyhow::anyhow!("{rejection:?}"))?;
    Ok(())
}

#[test]
fn store_manifest_round_trips() -> anyhow::Result<()> {
    let dir = Scratch::new()?;
    let (signing, verifying) = component_keypair();
    let mut value = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut value, base + 1, "0.1.0")?;
    store_manifest(dir.path(), &bytes, &signature)?;
    assert!(
        matches!(stored_pins(dir.path(), &verifying, base), Some(pins) if pins.serial == base + 1)
    );
    let leftovers = std::fs::read_dir(dir.path())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
        .count();
    assert_eq!(leftovers, 0);
    Ok(())
}
