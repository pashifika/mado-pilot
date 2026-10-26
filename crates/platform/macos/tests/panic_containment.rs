#![cfg_attr(not(target_os = "macos"), allow(missing_docs))]
#![cfg(target_os = "macos")]
//! Asserts why the callback trampolines contain their own panics.
//!
//! An unguarded Rust panic at the callback boundary aborts the process, so Rust
//! must contain it before returning to native code. Unit tests cover containment;
//! this test uses a child process to observe the unguarded form.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

/// Set on the child so it runs the unguarded call instead of spawning again.
const PROBE_VARIABLE: &str = "MADO_PILOT_MACOS_UNGUARDED_CALLBACK_PROBE";

const PROBE_TEST: &str = "an_unguarded_callback_panic_stops_its_process";

/// The shape the shim's callbacks would have without their containment.
///
/// # Safety
///
/// Called only by the child below, which exists to be stopped by it.
unsafe extern "C" fn unguarded_callback() {
    panic!("an unguarded extern \"C\" callback panicked");
}

#[test]
fn an_unguarded_callback_panic_stops_its_process() {
    if std::env::var_os(PROBE_VARIABLE).is_some() {
        // SAFETY: this is the child, and stopping here is the observation.
        unsafe { unguarded_callback() };
        unreachable!("an unwind out of an extern \"C\" function cannot continue");
    }

    let executable = std::env::current_exe().expect("the test binary is its own child");
    let child = Command::new(executable)
        .args(["--exact", PROBE_TEST, "--nocapture"])
        .env(PROBE_VARIABLE, "1")
        .output()
        .expect("spawning the probe child");

    assert!(
        !child.status.success(),
        "the unguarded form is expected to be fatal"
    );
    assert_eq!(
        child.status.code(),
        None,
        "the child was expected to be stopped by a signal rather than to exit: {:?}",
        child.status
    );
    assert!(
        child.status.signal().is_some(),
        "no terminating signal was reported: {:?}",
        child.status
    );
}
