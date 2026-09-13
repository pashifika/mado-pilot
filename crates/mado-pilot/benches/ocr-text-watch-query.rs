//! Model-free OCR scheduler workloads. No argument implicitly enables real OCR.
//!
//! The separately authorized process cohort is driven by
//! `tools/ocr-text-watch/workloads.py`; numeric enforcement is deliberately closed.

#[cfg(windows)]
#[path = "../examples/support/ocr_dependency_images.rs"]
mod ocr_dependency_images;

#[cfg(all(windows, target_arch = "x86_64", feature = "ocr-loader-diagnostic"))]
#[path = "support/ocr_loader_diagnostic.rs"]
mod ocr_loader_diagnostic;

#[path = "support/ocr_text_watch.rs"]
mod support;

use std::sync::mpsc;
use std::time::Duration;

use mado_pilot_testkit::bench_harness::{self, Accounting, Plan};

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

fn main() {
    #[cfg(feature = "ocr-loader-diagnostic")]
    {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            eprintln!("OCR loader diagnostic requires Windows x86_64");
            std::process::exit(2);
        }
        let mut arguments = std::env::args_os().skip(1);
        if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--semantic"))
            || arguments.next().is_some()
        {
            eprintln!("OCR loader diagnostic requires exactly --semantic");
            std::process::exit(2);
        }
    }

    #[cfg(not(feature = "ocr-loader-diagnostic"))]
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(not(feature = "ocr-loader-diagnostic"))]
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
    #[cfg(feature = "ocr-loader-diagnostic")]
    println!(
        "# loader diagnostic only; numerical_qualification=false; window=post-registration; pre-main=unobserved; historical_explanation=false"
    );
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
    #[cfg(all(windows, target_arch = "x86_64", feature = "ocr-loader-diagnostic"))]
    let diagnostic = match ocr_loader_diagnostic::Diagnostic::begin() {
        Ok(diagnostic) => diagnostic,
        Err(error) => {
            eprintln!("OCR loader diagnostic failed: {error}");
            std::process::exit(1);
        }
    };
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
    #[cfg(all(windows, not(feature = "ocr-loader-diagnostic")))]
    if let Err(error) = ocr_dependency_images::record_if_requested() {
        eprintln!(
            "OCR workload dependency observation failed: {error}; no dependency proof is implied"
        );
        std::process::exit(1);
    }
    #[cfg(all(windows, target_arch = "x86_64", feature = "ocr-loader-diagnostic"))]
    {
        let image_result = ocr_dependency_images::record_if_requested();
        let trace_result = diagnostic.finish(image_result.as_ref().err().map(|_| "end-image-writer-failed"));
        if let Err(error) = &image_result {
            eprintln!(
                "OCR workload dependency observation failed: {error}; no dependency proof is implied"
            );
        }
        if let Err(error) = &trace_result {
            eprintln!("OCR loader diagnostic failed: {error}");
        }
        if image_result.is_err() || trace_result.is_err() {
            std::process::exit(1);
        }
    }
    println!(
        "# cold-startup real_cpu=unexecuted controlled-construction-only=true; native_resources=unexecuted; task_8_2=not-passed task_8_3=not-passed"
    );
    finished.send(()).expect("watchdog remains alive");
    watchdog.join().expect("watchdog returned");
}
