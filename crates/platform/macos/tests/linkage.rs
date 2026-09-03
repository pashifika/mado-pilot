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

fn inspect_executable(tool: &str, arguments: &[&str]) -> String {
    link_the_shim();
    let executable = std::env::current_exe().expect("the test executable path is available");
    let output = Command::new(tool)
        .args(arguments)
        .arg(&executable)
        .output()
        .unwrap_or_else(|error| panic!("{tool} must inspect {}: {error}", executable.display()));
    assert!(
        output.status.success(),
        "{tool} {:?} failed for {}: {}",
        arguments,
        executable.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("{tool} returned non-UTF-8 output: {error}"))
}

fn load_commands() -> String {
    inspect_executable("/usr/bin/otool", &["-L"])
}

fn build_commands() -> String {
    inspect_executable("/usr/bin/otool", &["-l"])
}

fn linked_symbols() -> String {
    inspect_executable("/usr/bin/nm", &["-g"])
}

#[test]
fn the_final_artifact_declares_the_qualified_deployment_floor() {
    let commands = build_commands();

    assert!(
        commands.lines().any(|line| line.trim() == "minos 26.5.2"),
        "the final artifact does not declare macOS 26.5.2:\n{commands}"
    );
}

#[test]
fn controlled_frameworks_are_not_eager_dependencies() {
    let commands = load_commands();

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
    let commands = load_commands();

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
    let symbols = linked_symbols();

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
    let symbols = linked_symbols();

    assert!(
        !symbols.contains("_mp_fixture_"),
        "fixture-only control or event-recording symbols entered a production shim consumer"
    );
}

#[test]
fn private_fixture_shim_diagnostics_are_absent_from_production_consumers() {
    let symbols = linked_symbols();

    for symbol in [
        "_mp_shim_fixture_cleanup_counts",
        "_mp_shim_sck_diagnostics_set_tier",
        "_mp_shim_sck_diagnostics_dump",
    ] {
        assert!(
            !symbols
                .lines()
                .any(|line| line.split_whitespace().last() == Some(symbol)),
            "{symbol} entered a production shim consumer"
        );
    }
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
