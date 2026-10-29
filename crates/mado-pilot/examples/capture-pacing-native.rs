//! Private native pacing verification. Every execution requires explicit arguments.
//! The supervised parent grants capture/model authority and owns the outer deadline.

use std::io::Write;
use std::process::ExitCode;

#[path = "support/completion_cooldown.rs"]
mod completion_cooldown;
#[path = "support/capture_pacing_loop.rs"]
mod consumption;
#[path = "support/capture_pacing_contract.rs"]
mod contract;
#[path = "support/capture_pacing_control.rs"]
mod control;
#[path = "support/capture_pacing_metrics.rs"]
mod metrics;
#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(windows, target_arch = "x86_64", target_env = "msvc")
))]
#[path = "support/capture_pacing_native.rs"]
mod native;

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|_| eprintln!("consumer-panicked")));
    let mut report = contract::Report::new();
    let arguments = contract::Arguments::parse(std::env::args_os().skip(1));
    match arguments {
        Err(failure) => report.failed(failure),
        Ok(arguments) => {
            report.case = arguments.case.name();
            #[cfg(any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(windows, target_arch = "x86_64", target_env = "msvc")
            ))]
            {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    native::run(&arguments, &mut report)
                }));
                match outcome {
                    Ok(Ok(())) => report.semantic_status = "pass",
                    Ok(Err(failure)) => report.failed(failure),
                    Err(_) => {
                        report.cleanup_status = "fail";
                        report.failed(contract::Failure::Rule("consumer-panicked"));
                    }
                }
            }
            #[cfg(not(any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(windows, target_arch = "x86_64", target_env = "msvc")
            )))]
            report.failed(contract::Failure::NotRun("unsupported-host"));
        }
    }
    let success = report.semantic_status == "pass" && report.cleanup_status == "pass";
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    if serde_json::to_writer(&mut output, &report).is_err()
        || output.write_all(b"\n").is_err()
        || output.flush().is_err()
    {
        eprintln!("consumer-report-write");
        return ExitCode::FAILURE;
    }
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
