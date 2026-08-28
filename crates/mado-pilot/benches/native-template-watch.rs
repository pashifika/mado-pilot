//! Approved-host native template-watch qualification.
//!
//! The harness owns one repository fixture process, selects it through private
//! target-owner authentication, and drives visual state only through the bounded
//! fixture protocol. Every query uses the target facade constructor, a production
//! maintained capture session, and `Session::start_template_watch`.

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
    native_watch::run();

    #[cfg(not(any(windows, target_os = "macos")))]
    eprintln!("native-template-watch requires a declared MadoPilot release target");
}

#[cfg(any(windows, target_os = "macos"))]
#[path = "support/native_template_watch.rs"]
mod native_watch;
