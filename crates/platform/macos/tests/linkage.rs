#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Asserts deployment metadata, controlled framework linkage, and fixture
//! isolation.
//!
//! The qualified implementation floor is macOS 26.5.2 on Apple Silicon. The
//! workspace Cargo configuration owns the final Rust artifact's deployment target,
//! while the shim build repeats it for native object files. Frameworks opened
//! through controlled absolute paths — including fixture-only OpenGL — stay out of
//! production load commands.

use std::process::Command;

use mado_pilot_core::{OperationContext, PermissionProbe};
use mado_pilot_platform_macos::MacosPermissionProbe;

/// Frameworks the shim's build script declares eagerly.
const EXPECTED_FRAMEWORKS: [&str; 6] = [
    "ApplicationServices",
    "CoreFoundation",
    "CoreGraphics",
    "CoreMedia",
    "CoreVideo",
    "Foundation",
];

/// Frameworks whose absolute-path loading must not become eager load commands.
const DEFERRED_FRAMEWORKS: [&str; 3] = ["ScreenCaptureKit", "AppKit", "OpenGL"];

/// Process-directed entry points that must be resolved through the controlled
/// CoreGraphics loader rather than emitted as eager undefined references.
const DEFERRED_PROCESS_SYMBOLS: [&str; 2] = ["_CGEventPostToPid", "_CGPreflightPostEventAccess"];

/// Forces the shim into this binary's link.
///
/// A linker drops an archive nothing references, and then drops the framework
/// references that archive would have needed. Without this call the assertions
/// below would inspect a binary that never contained the boundary they are about,
/// and would pass for the wrong reason. The probe is the cheapest reference
/// available: it reads an existing authorization decision and prompts for nothing.
fn link_the_shim() {
    let _report = MacosPermissionProbe::new().report(&OperationContext::new());
}

fn load_commands() -> Option<String> {
    link_the_shim();
    let executable = std::env::current_exe().ok()?;
    let output = Command::new("otool")
        .arg("-L")
        .arg(&executable)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn build_commands() -> Option<String> {
    link_the_shim();
    let executable = std::env::current_exe().ok()?;
    let output = Command::new("otool")
        .arg("-l")
        .arg(&executable)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn linked_symbols() -> Option<String> {
    link_the_shim();
    let executable = std::env::current_exe().ok()?;
    let output = Command::new("/usr/bin/nm")
        .arg("-g")
        .arg(&executable)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[test]
fn the_final_artifact_declares_the_qualified_deployment_floor() {
    let Some(commands) = build_commands() else {
        println!("skipped: otool is unavailable, so build metadata cannot be inspected");
        return;
    };

    assert!(
        commands.lines().any(|line| line.trim() == "minos 26.5.2"),
        "the final artifact does not declare macOS 26.5.2:\n{commands}"
    );
}

#[test]
fn controlled_frameworks_are_not_eager_dependencies() {
    let Some(commands) = load_commands() else {
        println!("skipped: otool is unavailable, so load commands cannot be inspected");
        return;
    };

    for framework in DEFERRED_FRAMEWORKS {
        assert!(
            !commands.contains(&format!("{framework}.framework")),
            "a binary linking this Adapter must not depend on {framework} at load time:\n\
             {commands}"
        );
    }
}

#[test]
fn the_frameworks_the_shim_needs_at_load_are_declared_by_its_build_script() {
    let Some(commands) = load_commands() else {
        println!("skipped: otool is unavailable, so load commands cannot be inspected");
        return;
    };

    for framework in EXPECTED_FRAMEWORKS {
        assert!(
            commands.contains(&format!("{framework}.framework")),
            "{framework} is missing from the load commands, so the build script's \
             framework declarations are not reaching the final link:\n{commands}"
        );
    }
}

#[test]
fn process_post_symbols_are_not_eager_link_dependencies() {
    let Some(symbols) = linked_symbols() else {
        println!("skipped: nm is unavailable, so linked symbols cannot be inspected");
        return;
    };

    for symbol in DEFERRED_PROCESS_SYMBOLS {
        assert!(
            !symbols
                .lines()
                .any(|line| line.split_whitespace().last() == Some(symbol)),
            "{symbol} must be loaded through the controlled CoreGraphics boundary:\n{symbols}"
        );
    }
}

#[test]
fn the_fixture_control_archive_is_absent_from_a_production_shim_consumer() {
    let Some(symbols) = linked_symbols() else {
        println!("skipped: nm is unavailable, so linked symbols cannot be inspected");
        return;
    };

    assert!(
        !symbols.contains("_mp_fixture_"),
        "fixture-only control or event-recording symbols entered a production shim consumer"
    );
}

#[test]
fn normal_package_targets_gate_the_private_fixture_binary() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata is valid JSON");
    let fixture = metadata["packages"]
        .as_array()
        .expect("packages is an array")
        .iter()
        .flat_map(|package| package["targets"].as_array().expect("targets is an array"))
        .find(|target| target["name"] == "mado-pilot-macos-input-fixture")
        .expect("the private fixture target is declared");
    assert_eq!(
        fixture["required-features"],
        serde_json::json!(["private-fixture"]),
        "the fixture binary must require the explicit private-fixture feature"
    );
}
