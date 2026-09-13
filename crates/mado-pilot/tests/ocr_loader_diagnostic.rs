//! Synthetic loader-notification contracts, not a real loader observation.
//!
//! This runnable integration harness never registers a notification callback,
//! snapshots the process, forces a module, or invokes an OCR benchmark. Execution
//! still requires the separate isolated synthetic-validation grant.

#![cfg(all(windows, target_arch = "x86_64", feature = "ocr-loader-diagnostic"))]

#[path = "../examples/support/ocr_image_path.rs"]
mod ocr_image_path;
#[path = "../benches/support/ocr_loader_recording.rs"]
mod ocr_loader_recording;

use std::ffi::c_void;
use std::ptr::addr_of;
use std::sync::mpsc;
use std::time::Duration;

use ocr_loader_recording::{
    Failure, NotificationData, NotificationPayload, Report, Storage, UnicodeString,
    mado_pilot_ocr_loader_notification,
};
use serde_json::Value;

const OUTPUT_LIMIT: usize = 1_048_576;

// Synthetic-only helpers: no registration, process snapshot, module loading or
// real observation. Each test supplies a distinct process-lifetime static arena.
struct Recorder {
    storage: Storage,
}

impl Recorder {
    const fn new() -> Self {
        Self {
            storage: Storage::new(),
        }
    }

    fn notify(&'static self, reason: u32, path: &[u16]) {
        let length = u16::try_from(path.len() * 2).expect("bounded synthetic path");
        self.notify_length(reason, path, length);
    }

    fn notify_length(&'static self, reason: u32, path: &[u16], length: u16) {
        assert!(usize::from(length) <= path.len() * 2);
        let name = UnicodeString {
            length,
            maximum_length: u16::try_from(path.len() * 2).expect("bounded synthetic path"),
            buffer: path.as_ptr(),
        };
        let data = NotificationData {
            loaded: NotificationPayload {
                flags: 0,
                full_dll_name: &name,
                base_dll_name: &name,
                dll_base: std::ptr::without_provenance_mut(0x1000),
                size_of_image: 4096,
            },
        };
        // SAFETY: Synthetic borrowed ABI objects and their buffers remain
        // live throughout the call; only the declared bounded length is read.
        // The context is a static arena, exactly as in production.
        unsafe {
            mado_pilot_ocr_loader_notification(
                reason,
                &data,
                addr_of!(self.storage).cast_mut().cast::<c_void>(),
            );
        }
    }

    fn notify_null(&'static self) {
        // SAFETY: The permanent context is valid; a null payload is rejected
        // before dereference by the actual callback payload validation.
        unsafe {
            mado_pilot_ocr_loader_notification(
                1,
                std::ptr::null(),
                addr_of!(self.storage).cast_mut().cast::<c_void>(),
            );
        }
    }

    fn hold_writer(&'static self) -> HeldWriter {
        assert!(self.storage.admit());
        HeldWriter {
            storage: &self.storage,
        }
    }

    fn close(&self) {
        self.storage.close();
    }

    fn drained(&self) -> bool {
        self.storage.drain(Duration::ZERO)
    }

    fn report(&self, status: Option<i32>, limit: usize) -> (Vec<u8>, Result<(), Failure>) {
        let mut bytes = Vec::new();
        let result = (|| {
            let mut report = Report::new(&mut bytes, limit)?;
            let image = (0x1000, "C:\\synthetic\\fixture.exe");
            report.checkpoint("initial", 0, 0, [image])?;
            let observed = self.storage.observed();
            report.checkpoint("final", observed, observed, [image])?;
            report.finish(
                &self.storage,
                status,
                self.storage.drain(Duration::ZERO),
                None,
            )
        })();
        (bytes, result)
    }
}

struct HeldWriter {
    storage: &'static Storage,
}

impl Drop for HeldWriter {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful unmatched admission made by
        // hold_writer. It never accesses slots and releases its admission
        // exactly once on drop, with no slot accesses remaining.
        unsafe {
            self.storage.release();
        }
    }
}

fn decoded(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("complete synthetic report JSON")
}

#[test]
fn zero_notifications_are_a_complete_observation_not_numeric_qualification() {
    static RECORDER: Recorder = Recorder::new();
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    result.expect("zero notifications are valid");
    let report = decoded(&bytes);
    assert_eq!(report["complete"], true);
    assert_eq!(report["observed_events"], 0);
    assert_eq!(report["events"], serde_json::json!([]));
    assert_eq!(report["error"], Value::Null);
    assert_eq!(report["callbacks_drained"], true);
    assert_eq!(report["numerical_qualification"], false);
    assert_eq!(report["pre_main_history"], false);
    assert_eq!(report["scope"], "post-registration-window");
}

#[test]
fn notification_bytes_survive_borrowed_buffer_mutation_and_drop() {
    static RECORDER: Recorder = Recorder::new();
    let expected = "C:\\synthetic\\owned-\u{1d11e}.dll";
    {
        let mut borrowed: Vec<u16> = expected.encode_utf16().collect();
        RECORDER.notify(1, &borrowed);
        borrowed.fill(u16::from(b'X'));
    }
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    result.expect("owned callback bytes remain valid after the borrow ends");
    let report = decoded(&bytes);
    assert_eq!(report["events"][0]["path"], expected);
    assert_eq!(report["events"][0]["kind"], "load");
    assert_eq!(report["complete"], true);
}

#[test]
fn malformed_payloads_fail_without_losing_prior_valid_events() {
    static RECORDER: Recorder = Recorder::new();
    let valid: Vec<u16> = "C:\\synthetic\\retained.dll".encode_utf16().collect();
    RECORDER.notify(1, &valid);
    let mut bad_encoding: Vec<u16> = "C:\\synthetic\\".encode_utf16().collect();
    bad_encoding.push(0xd800);
    RECORDER.notify(1, &bad_encoding);
    RECORDER.notify_length(1, &valid, 1);
    RECORDER.notify_null();
    RECORDER.notify(3, &valid);
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    assert!(result.is_err());
    let report = decoded(&bytes);
    assert_eq!(report["complete"], false);
    assert_eq!(report["malformed"], true);
    assert_eq!(report["observed_events"], 5);
    assert_eq!(report["events"].as_array().expect("event array").len(), 1);
    assert_eq!(report["events"][0]["path"], "C:\\synthetic\\retained.dll");
    let error = report["error"].as_str().expect("bounded failure rule");
    assert!(error.len() <= 64 && !error.contains(['\\', '/', ':', '\n', '\r']));
}

#[test]
fn paths_allow_exactly_2048_utf16_units_and_reject_overlong_without_truncation() {
    static EXACT: Recorder = Recorder::new();
    static OVERLONG: Recorder = Recorder::new();
    let path = format!("C:\\{}x", "a\\".repeat(1022));
    let mut units: Vec<u16> = path.encode_utf16().collect();
    assert_eq!(units.len(), 2048);
    EXACT.notify(2, &units);
    EXACT.close();
    let (bytes, result) = EXACT.report(Some(0), OUTPUT_LIMIT);
    result.expect("the declared content boundary is accepted");
    let exact = decoded(&bytes);
    assert_eq!(exact["events"][0]["path"], path);
    assert_eq!(exact["events"][0]["kind"], "unload");

    units.push(u16::from(b'z'));
    OVERLONG.notify(1, &units);
    OVERLONG.close();
    let (bytes, result) = OVERLONG.report(Some(0), OUTPUT_LIMIT);
    assert!(result.is_err());
    let overlong = decoded(&bytes);
    assert_eq!(overlong["complete"], false);
    assert_eq!(overlong["malformed"], true);
    assert_eq!(overlong["observed_events"], 1);
    assert_eq!(overlong["events"], serde_json::json!([]));
}

#[test]
fn the_257th_notification_cannot_overwrite_the_256_owned_slots() {
    static RECORDER: Recorder = Recorder::new();
    let path: Vec<u16> = "C:\\synthetic\\bounded.dll".encode_utf16().collect();
    for _ in 0..256 {
        RECORDER.notify(1, &path);
    }
    RECORDER.notify(
        2,
        &"C:\\synthetic\\overflow.dll"
            .encode_utf16()
            .collect::<Vec<_>>(),
    );
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    assert!(result.is_err());
    let report = decoded(&bytes);
    assert_eq!(report["complete"], false);
    assert_eq!(report["overflow"], true);
    assert_eq!(report["observed_events"], 257);
    let events = report["events"].as_array().expect("retained event array");
    assert_eq!(events.len(), 256);
    assert_eq!(events[0]["ordinal"], 1);
    assert_eq!(events[255]["ordinal"], 256);
    assert!(
        events
            .iter()
            .all(|event| event["path"] == "C:\\synthetic\\bounded.dll")
    );
}

#[test]
fn the_fixed_mebibyte_cap_retains_a_partial_report_and_fails() {
    static RECORDER: Recorder = Recorder::new();
    let path: Vec<u16> = format!("C:\\{}", "界".repeat(2045))
        .encode_utf16()
        .collect();
    for _ in 0..256 {
        RECORDER.notify(1, &path);
    }
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    assert!(result.is_err());
    assert!(bytes.len() <= OUTPUT_LIMIT);
    // The failed write must not clear earlier checkpoints/events or exceed the
    // cap. One maximal UTF8 path plus record framing fits well within this gap.
    assert!(bytes.len() > OUTPUT_LIMIT - 16_384);
    assert!(serde_json::from_slice::<Value>(&bytes).is_err());
}

#[test]
fn closing_admission_does_not_make_a_held_writer_drained_or_its_slots_readable() {
    static RECORDER: Recorder = Recorder::new();
    RECORDER.notify(
        1,
        &"C:\\synthetic\\before-hold.dll"
            .encode_utf16()
            .collect::<Vec<_>>(),
    );
    let (ready, admitted) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let held = RECORDER.hold_writer();
        ready.send(()).expect("writer admitted");
        released.recv().expect("explicit writer release");
        drop(held);
    });
    admitted
        .recv()
        .expect("held writer established before close");
    RECORDER.close();
    let drained_while_held = RECORDER.drained();
    let (partial_bytes, partial_result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    // Always release/join before assertions, so an assertion failure cannot leave
    // a blocked test thread behind or manufacture a timeout-based race test.
    release.send(()).expect("release held writer");
    writer.join().expect("held writer returned");
    assert!(!drained_while_held);
    assert!(partial_result.is_err());
    let partial = decoded(&partial_bytes);
    assert_eq!(partial["admission_closed"], true);
    assert_eq!(partial["callbacks_drained"], false);
    assert_eq!(partial["unregistered"], true);
    assert_eq!(partial["complete"], false);
    // Not even an earlier committed slot is inspected until every writer drains.
    assert_eq!(partial["events"], serde_json::json!([]));
    assert!(RECORDER.drained());
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    result.expect("the actual writer fence has now completed");
    assert_eq!(
        decoded(&bytes)["events"][0]["path"],
        "C:\\synthetic\\before-hold.dll"
    );
}

#[test]
fn late_entry_after_close_cannot_reopen_or_mutate_the_observation() {
    static RECORDER: Recorder = Recorder::new();
    RECORDER.notify(
        1,
        &"C:\\synthetic\\before-close.dll"
            .encode_utf16()
            .collect::<Vec<_>>(),
    );
    RECORDER.close();
    let late = std::thread::spawn(|| {
        RECORDER.notify(
            2,
            &"C:\\synthetic\\late.dll".encode_utf16().collect::<Vec<_>>(),
        );
    });
    late.join()
        .expect("late callback returns without slot access");
    let (bytes, result) = RECORDER.report(Some(0), OUTPUT_LIMIT);
    result.expect("late entry is excluded by the closed admission gate");
    let report = decoded(&bytes);
    assert_eq!(report["observed_events"], 1);
    assert_eq!(report["events"].as_array().expect("event array").len(), 1);
    assert_eq!(
        report["events"][0]["path"],
        "C:\\synthetic\\before-close.dll"
    );
    assert_eq!(report["callbacks_drained"], true);
}

#[test]
fn failed_unregister_retains_drained_evidence_and_never_claims_completion() {
    static RECORDER: Recorder = Recorder::new();
    RECORDER.notify(
        1,
        &"C:\\synthetic\\retained-after-failure.dll"
            .encode_utf16()
            .collect::<Vec<_>>(),
    );
    RECORDER.close();
    let (bytes, result) = RECORDER.report(Some(-1), OUTPUT_LIMIT);
    assert!(result.is_err());
    let report = decoded(&bytes);
    assert_eq!(report["complete"], false);
    assert_eq!(report["unregistered"], false);
    assert_eq!(report["unregister_status"], -1);
    assert_eq!(report["callbacks_drained"], true);
    assert_eq!(
        report["events"][0]["path"],
        "C:\\synthetic\\retained-after-failure.dll"
    );
    // Storage still exists after failure; a hypothetical late native entry must
    // return through the closed gate rather than touch freed callback context.
    RECORDER.notify_null();
    assert!(RECORDER.drained());
}

#[test]
fn absent_unregister_result_is_nonpass_not_an_invented_ntstatus() {
    static RECORDER: Recorder = Recorder::new();
    RECORDER.close();
    let (bytes, result) = RECORDER.report(None, OUTPUT_LIMIT);
    assert!(result.is_err());
    let report = decoded(&bytes);
    assert_eq!(report["complete"], false);
    assert_eq!(report["unregistered"], false);
    assert_eq!(report["unregister_status"], Value::Null);
    assert_eq!(report["callbacks_drained"], true);
}
