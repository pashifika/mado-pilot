#![cfg(target_os = "macos")]
//! Deterministic regressions for the benchmark-only macOS fixture controller.

use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/macos_fixture.rs"]
mod macos_fixture;
