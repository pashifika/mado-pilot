//! Runs the benchmark-only macOS fixture controller's deterministic support tests.
#![cfg(target_os = "macos")]
#![allow(dead_code, unreachable_pub, unused_imports)]

use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

#[path = "../benches/support/macos_fixture.rs"]
mod macos_fixture;
