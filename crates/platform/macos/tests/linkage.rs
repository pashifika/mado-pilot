//! Asserts that nothing newer than the deployment minimum is linked eagerly.
//!
//! ADR 0012 requires that a capability newer than the declared minimum macOS
//! version reports a status on an unsupported host rather than failing to load.
//! The mechanism the shim uses is controlled dynamic loading rather than the
//! `-weak_framework` the ADR named, because Cargo does not propagate a
//! dependency's `rustc-link-arg` to the binary that consumes the dependency. This
//! test pins the property that matters either way: a binary that links this
//! Adapter carries no load command for the capture framework, so the dynamic
//! loader has nothing to fail on.
//!
//! What this cannot do is exercise the unsupported host itself. The framework is
//! present here, and no host without it is available to this repository's
//! verification; that limit is recorded rather than glossed.

#![cfg(target_os = "macos")]

use std::process::Command;

use mado_pilot_core::{OperationContext, PermissionProbe};
use mado_pilot_platform_macos::MacosPermissionProbe;

/// Frameworks the shim's build script declares. Every one of them predates any
/// macOS version this project could select as its minimum.
const EXPECTED_FRAMEWORKS: [&str; 6] = [
    "ApplicationServices",
    "CoreFoundation",
    "CoreGraphics",
    "CoreMedia",
    "CoreVideo",
    "Foundation",
];

/// The framework that arrived in macOS 12.3 and must not be a load command.
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
