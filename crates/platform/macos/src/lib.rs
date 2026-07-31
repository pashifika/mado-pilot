//! Native macOS target discovery, permission probes, and ScreenCaptureKit capture.
//!
//! This package is the only workspace package that names ScreenCaptureKit, Core
//! Video, Core Graphics, Accessibility, or Objective-C. The platform-neutral
//! capture contracts depend on none of them, and no Objective-C type appears in
//! any Rust or public API.
//!
//! The implementation is target-gated: off macOS this crate keeps an empty,
//! documented seam and resolves no macOS dependency. macOS builds expose
//! [`MacosCaptureProvider`] for picker-free discovery and capture, and
//! [`MacosPermissionProbe`] for the two authorizations macOS grants separately.
//!
//! # The native boundary
//!
//! One internal C-callable surface, implemented in Objective-C with Automatic
//! Reference Counting and compiled with `-fobjc-arc-exceptions`, exactly as
//! [ADR 0012](../../../docs/adr/0012-macos-shim-language-and-containment.md)
//! records: every entry point and callback trampoline contains native exceptions,
//! every Rust callback contains its own panics, each frame work item pools its own
//! temporaries, callback admission is fenced by disable-and-drain before the caller
//! releases registered state, and close is idempotent and completes its release
//! even when it reports a failure.
//!
//! ScreenCaptureKit is loaded from its absolute system location at runtime rather
//! than linked. That is a change of mechanism from the `-weak_framework` the ADR
//! named, for a reason the prototype could not observe: Cargo does not propagate a
//! dependency's `rustc-link-arg` to the binary that consumes it, so a build script
//! here cannot own that flag. The property the ADR asked for is unchanged — a host
//! below the framework's minimum reports an unsupported status rather than failing
//! to load — and `tests/linkage.rs` asserts it. The minimum supported macOS
//! version itself remains gate `G-001`.
//!
//! # Authorization
//!
//! Nothing here requests a permission or presents permission UI. Discovery and
//! open preflight Screen Recording with the non-prompting Core Graphics check and
//! refuse before reaching the framework query that would otherwise show the system
//! dialog. Screen Recording and Accessibility are reported separately, and neither
//! stands in for the other.
//!
//! # Ownership
//!
//! A producer surface belongs to a pool of fixed depth, so the callback copies a
//! frame's content into Adapter-owned Core Video storage from a finite budget and
//! publishes that. A retained public frame therefore pins nothing capture needs to
//! make progress, and CPU conversion happens only under an explicit mapping.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "macos")]
mod availability;
#[cfg(target_os = "macos")]
mod discovery;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
mod permission;
#[cfg(target_os = "macos")]
mod provider;
#[cfg(all(test, target_os = "macos"))]
mod scenarios;
#[cfg(target_os = "macos")]
mod shim;
#[cfg(target_os = "macos")]
mod storage;

#[cfg(target_os = "macos")]
pub use permission::MacosPermissionProbe;
#[cfg(target_os = "macos")]
pub use provider::{MacosCaptureProvider, PROVIDER};
