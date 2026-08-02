#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]

//! Keychain-independent structural verification for the dedicated input fixture.
//!
//! The test copies generated build output into a private temporary bundle, ad-hoc
//! signs only that copy, asks `codesign` to verify its structure, and executes a
//! metadata-only reporting mode. It never creates a window, requests permission,
//! reads TCC, changes focus, or posts input.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use mado_pilot_platform_macos::fixture_protocol::BUNDLE_IDENTIFIER;

struct GeneratedBundle(PathBuf);

impl Drop for GeneratedBundle {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the qualified macOS tool runs")
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn generated_bundle() -> (GeneratedBundle, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the host clock is after the epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "mado-pilot-macos-input-signing-{}-{nonce}",
        std::process::id()
    ));
    let bundle = root.join("MadoPilotInputFixture.app");
    let macos = bundle.join("Contents/MacOS");
    fs::create_dir_all(&macos).expect("the generated bundle directory is created");

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::copy(
        manifest.join("bundle/Info.plist"),
        bundle.join("Contents/Info.plist"),
    )
    .expect("the tracked fixture metadata is copied");
    let executable = macos.join("mado-pilot-macos-input-fixture");
    fs::copy(
        env!("CARGO_BIN_EXE_mado-pilot-macos-input-fixture"),
        &executable,
    )
    .expect("the generated fixture executable is copied");

    (GeneratedBundle(root), executable)
}

#[test]
fn generated_fixture_bundle_is_ad_hoc_signed_verified_and_truthfully_reported() {
    let (generated, executable) = generated_bundle();
    let bundle = generated.0.join("MadoPilotInputFixture.app");

    // Identity `-` is ad-hoc signing: it does not name or consult a certificate
    // identity in the user's keychain. `--force` is confined to this generated
    // temporary bundle and can overwrite no repository or user artifact.
    let signed = run(Command::new("/usr/bin/codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg("--identifier")
        .arg(BUNDLE_IDENTIFIER)
        .arg("--timestamp=none")
        .arg(&bundle));
    assert_success(&signed, "ad-hoc fixture signing");

    let verified = run(Command::new("/usr/bin/codesign")
        .arg("--verify")
        .arg("--strict")
        .arg("--verbose=2")
        .arg(&bundle));
    assert_success(&verified, "structural fixture signature verification");

    let reported = run(Command::new(&executable).arg("--report-execution-context"));
    assert_success(&reported, "fixture execution-context reporting");
    assert_eq!(
        String::from_utf8(reported.stdout)
            .expect("the fixture report is UTF-8")
            .trim(),
        format!(
            "fixture-context launch=bundled signature=ad-hoc signing-identifier={BUNDLE_IDENTIFIER}"
        )
    );
}
