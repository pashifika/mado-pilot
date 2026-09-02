//! Deterministic coverage for bounded native root-cause diagnostics.
#![cfg(all(target_os = "macos", feature = "native-template-watch-qualification"))]

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/macos_fixture.rs"]
mod macos_fixture;
use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/native_template_watch.rs"]
mod native_watch;
