//! Model-free OCR scheduler workloads. No argument implicitly enables real OCR.
//!
//! The separately authorized process cohort is driven by
//! `tools/ocr-text-watch/workloads.py`; numeric enforcement is deliberately closed.

#[cfg(windows)]
#[path = "../examples/support/ocr_dependency_images.rs"]
mod ocr_dependency_images;

#[path = "support/ocr_text_watch.rs"]
mod support;

use std::sync::mpsc;
use std::time::Duration;

use mado_pilot_testkit::bench_harness::{self, Accounting, Plan};

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    for argument in &arguments {
        match argument.as_str() {
            "--test" | "--bench" | "--semantic" => {}
            "--enforce-budgets" => {
                eprintln!(
                    "OCR numeric enforcement refused: no accepted target-specific budget ADR exists"
                );
                std::process::exit(2);
            }
            _ => {
                eprintln!(
                    "OCR workload arguments: --semantic, --test, --bench; real CPU requires the explicit Python cohort procedure"
                );
                std::process::exit(2);
            }
        }
    }

    // A broken join must also be finite under ordinary `cargo test --all-targets`.
    // This is a driver watchdog, not an engine analysis worker or latency budget.
    let (finished, completion) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if matches!(
            completion.recv_timeout(Duration::from_secs(300)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!("OCR workload process exceeded its prospective 300-second bound");
            std::process::exit(1);
        }
    });
    println!(
        "# lane=controlled-scheduler semantics-only=true model_execution=false native_capture=false numeric_budgets=unaccepted"
    );
    println!(
        "# source=generated-solid-bgra extent=960x540 roi=full interval_ms=50 capture_tick_ms=16 clock=manual-query/Instant-driver"
    );
    println!(
        "# elapsed values include driver/oracles/gates; they are not CPU OCR inference latency"
    );
    println!(
        "# mapped_bytes_per_result=largest-observed-backend-input; total-physical-mapping-bytes=unmeasured-nonpass"
    );
    let plan = Plan::new(2, 20);
    let workloads = support::workloads(plan);
    let (startup, repeated) = workloads.split_last().expect("seven fixed workloads");
    bench_harness::summarize("ocr-text-watch-query", plan, repeated);
    bench_harness::summarize(
        "ocr-text-watch-controlled-startup",
        Plan::new(0, 1),
        std::slice::from_ref(startup),
    );
    for workload in &workloads {
        assert_eq!(workload.incorrect(), 0, "semantic workload failed");
    }
    #[cfg(windows)]
    if let Err(error) = ocr_dependency_images::record_if_requested() {
        eprintln!(
            "OCR workload dependency observation failed: {error}; no dependency proof is implied"
        );
        std::process::exit(1);
    }
    println!(
        "# cold-startup real_cpu=unexecuted controlled-construction-only=true; native_resources=unexecuted; task_8_2=not-passed task_8_3=not-passed"
    );
    finished.send(()).expect("watchdog remains alive");
    watchdog.join().expect("watchdog returned");
}
