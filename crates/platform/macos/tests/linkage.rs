#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Asserts the deployment metadata and controlled framework linkage.
//!
//! The qualified implementation floor is macOS 26.5.2 on Apple Silicon. The
//! workspace Cargo configuration owns the final Rust artifact's deployment target,
//! while the shim build repeats it for native object files. ScreenCaptureKit stays
//! controlled-loaded from its absolute system path, so no ambient or eager framework
//! resolution enters the final binary.

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

/// The framework whose absolute-path loading must not become an eager load command.
const DEFERRED_FRAMEWORK: &str = "ScreenCaptureKit";

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
fn the_capture_framework_is_not_an_eager_dependency() {
    let Some(commands) = load_commands() else {
        println!("skipped: otool is unavailable, so load commands cannot be inspected");
        return;
    };

    assert!(
        !commands.contains(DEFERRED_FRAMEWORK),
        "a binary linking this Adapter must not depend on {DEFERRED_FRAMEWORK} \
         at load time:\n{commands}"
    );
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
