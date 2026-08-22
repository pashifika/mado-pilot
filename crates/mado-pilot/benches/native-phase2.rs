//! Phase 2 native capture, input, lifecycle, and common-flow measurements.
//!
//! This benchmark owns a repository fixture process and refuses any other
//! target. macOS capture and transition profiles change deterministic fixture
//! state only through the bounded private command plane; acknowledgements remain
//! separate from strictly newer-frame oracles. The interactive `System` input
//! set and Windows profile lineage retain their existing product-input stimulus.
//!
//! Ordinary `cargo test --all-targets` compiles this target and exits before it
//! opens a native capability. A measurement is explicit because it needs an
//! interactive, authorized release-target desktop and operator-supplied profile
//! conditions.
#[cfg(target_os = "macos")]
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "support/macos_fixture.rs"]
mod macos_fixture;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

use mado_pilot_testkit::bench_harness::Accounting;

#[global_allocator]
static ACCOUNTING: Accounting = Accounting;

fn main() {
    #[cfg(any(windows, target_os = "macos"))]
    native::run();

    #[cfg(not(any(windows, target_os = "macos")))]
    eprintln!("native-phase2 requires a declared MadoPilot release target");
}

#[cfg(any(windows, target_os = "macos"))]
#[path = "support/native_phase2.rs"]
mod native;
