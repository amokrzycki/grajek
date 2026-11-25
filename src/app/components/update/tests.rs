use super::{RemoteCheck, outcome, update_check_visible};
use crate::{
    media::components::{
        Mode, Need,
        channel::{self, Accepted, CheckError, Rejection, fetch},
        embedded_pins,
    },
    test_support::{
        Scratch, component_keypair, component_manifest, embedded_serial, server, signed_manifest,
    },
};

#[test]
fn update_check_visibility_rules() {
    assert!(update_check_visible(Mode::Managed, None, false));
    assert!(update_check_visible(
        Mode::System,
        Some(Need::Missing),
        false
    ));
    assert!(!update_check_visible(Mode::System, None, false));
    assert!(!update_check_visible(Mode::Managed, None, true));
    assert!(!update_check_visible(
        Mode::System,
        Some(Need::Missing),
        true
    ));
}

#[tokio::test]
async fn missing_manifest_reports_a_retryable_failure() -> anyhow::Result<()> {
    let server = server(vec![]).await?;
    let scratch = Scratch::new()?;
    let result = fetch::check(&reqwest::Client::new(), &server.base, scratch.path()).await;
    assert_eq!(
        outcome(result, None, embedded_pins()),
        RemoteCheck::Failed("Couldn't check for updates. Try again later.")
    );
    assert_eq!(std::fs::read_dir(scratch.path())?.count(), 0);
    Ok(())
}

#[test]
fn newer_serial_with_same_versions_is_up_to_date() -> anyhow::Result<()> {
    let scratch = Scratch::new()?;
    let (signing, key) = component_keypair();
    let mut manifest = component_manifest()?;
    let base = embedded_serial()?;
    let (bytes, signature) = signed_manifest(&signing, &mut manifest, base + 1, "0.1.0")?;
    assert_eq!(
        channel::accept(scratch.path(), &bytes, &signature, &key),
        Ok(Accepted::Stored)
    );
    assert!(scratch.file("component-manifest.json").exists());
    assert_eq!(
        outcome(Ok(Accepted::Stored), None, embedded_pins()),
        RemoteCheck::UpToDate
    );
    let pins = embedded_pins()?;
    assert_eq!(
        outcome(Ok(Accepted::Current), Some(Need::Update), embedded_pins()),
        RemoteCheck::Available {
            ytdlp: pins.ytdlp.version,
            deno: pins.deno.version,
        }
    );
    assert_eq!(
        outcome(
            Err(CheckError::Rejected(Rejection::BadSignature)),
            None,
            embedded_pins()
        ),
        RemoteCheck::Failed("The update didn't pass verification and was ignored.")
    );
    Ok(())
}
