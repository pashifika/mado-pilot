//! Deterministic foreign-controller protocol tests; no native fixture is launched.
#![cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-template-watch-qualification"
))]

#[cfg(target_os = "macos")]
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/macos_fixture.rs"]
mod macos_fixture;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_control as macos_fixture_control;
#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_protocol as macos_fixture_protocol;

#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../benches/support/native_template_watch.rs"]
mod native_watch;
