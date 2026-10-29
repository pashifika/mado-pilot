//! Settlement and process containment for native concurrency tests.

use std::process::Command;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mado_pilot_testkit::bench_harness::{self, BoundedChildOutput};

use crate::inference::test_hook::{self, RunGateGuard};

const CHILD_ENV: &str = "MADO_PILOT_ONNX_NATIVE_TEST_CHILD";
// A whole-test hang fuse, not an OCR latency or cancellation-response budget.
// Includes backend opens, the contract suite, repeated oracles and cleanup.
const NATIVE_TEST_BOUND: Duration = Duration::from_secs(300);
const OUTPUT_LIMIT: usize = 256 * 1024;

fn child_command(test: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", test, "--include-ignored", "--nocapture"])
        .env(CHILD_ENV, test);
    command
}

fn child_output(test: &str, bound: Duration) -> BoundedChildOutput {
    bench_harness::bounded_child_output(&mut child_command(test), bound, OUTPUT_LIMIT)
}

/// Returns true in the parent after the exact test has completed in its child.
/// Keep the parent's serialization guard held across this call as well.
pub(super) fn run_in_child() -> bool {
    let current = thread::current();
    let test = current.name().expect("native test has a harness name");
    if std::env::var(CHILD_ENV).as_deref() == Ok(test) {
        return false;
    }
    let output = child_output(test, NATIVE_TEST_BOUND);
    assert!(
        output.within_bounds && output.status.is_some_and(|status| status.success()),
        "native test {test} failed (whole-test bound {NATIVE_TEST_BOUND:?}, \
         within_bounds={}, status={:?})\nstdout:\n{}\nstderr:\n{}",
        output.within_bounds,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    true
}

/// Owns all resources that must outlive a gated inference worker, including
/// during assertion unwind. Only use inside a bounded native-test child: a
/// thread stuck in native code cannot be safely terminated in-process.
pub(super) struct NativeRun<T> {
    pub(super) gate: RunGateGuard,
    receiver: mpsc::Receiver<T>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> NativeRun<T> {
    pub(super) fn spawn(run: impl FnOnce() -> T + Send + 'static) -> Self {
        let current = thread::current();
        let test = current.name().expect("native worker has a harness name");
        assert_eq!(
            std::env::var(CHILD_ENV).as_deref(),
            Ok(test),
            "native workers require a bounded test child"
        );
        let gate = test_hook::install();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            assert!(sender.send(run()).is_ok(), "worker receiver remains live");
        });
        Self {
            gate,
            receiver,
            worker: Some(worker),
        }
    }

    pub(super) fn recv_timeout(&self, bound: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(bound)
    }

    pub(super) fn join(mut self) -> thread::Result<()> {
        self.worker.take().expect("worker joins once").join()
    }

    /// Full OCR completion is bounded by the child process, not the gate's
    /// handshake budget. Join before inspecting a disconnected result channel.
    pub(super) fn finish(mut self) -> T {
        let result = self.receiver.recv();
        self.worker
            .take()
            .expect("worker joins once")
            .join()
            .expect("native inference worker joins");
        result.expect("native inference publishes a result")
    }
}

impl<T> Drop for NativeRun<T> {
    fn drop(&mut self) {
        self.gate.release();
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            // Preserve the original assertion panic, rather than aborting with
            // a second panic. Normal destruction still surfaces worker failure.
            if !thread::panicking()
                && let Err(panic) = result
            {
                std::panic::resume_unwind(panic);
            }
        }
        // The receiver and active gate are dropped only after the worker exits.
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::native_tests::{NATIVE_GATE_BOUND, serialize_native_runtime_test};

    #[test]
    fn completion_may_outlast_the_gate_handshake_budget() {
        let _serial = serialize_native_runtime_test();
        if run_in_child() {
            return;
        }
        let run = NativeRun::spawn(|| {
            test_hook::at_run_admission();
            thread::sleep(NATIVE_GATE_BOUND + Duration::from_millis(50));
            42
        });
        assert!(run.gate.wait_until_admitted(NATIVE_GATE_BOUND));
        run.gate.release();
        assert_eq!(run.finish(), 42);
    }

    #[test]
    fn assertion_unwind_settles_worker_before_reusing_gate() {
        let _serial = serialize_native_runtime_test();
        if run_in_child() {
            return;
        }
        struct DeliveredResult {
            receiver_thread: thread::ThreadId,
            dropped_by_receiver: Arc<AtomicBool>,
        }
        impl Drop for DeliveredResult {
            fn drop(&mut self) {
                self.dropped_by_receiver.store(
                    thread::current().id() == self.receiver_thread,
                    Ordering::SeqCst,
                );
            }
        }
        for panic_worker in [false, true] {
            let dropped_by_receiver = Arc::new(AtomicBool::new(false));
            let result = DeliveredResult {
                receiver_thread: thread::current().id(),
                dropped_by_receiver: Arc::clone(&dropped_by_receiver),
            };
            let settled = Arc::new(AtomicBool::new(false));
            let worker_settled = Arc::clone(&settled);
            let failure = std::panic::catch_unwind(move || {
                let run = NativeRun::spawn(move || {
                    test_hook::at_run_admission();
                    worker_settled.store(true, Ordering::SeqCst);
                    assert!(!panic_worker, "injected worker failure");
                    result
                });
                assert!(run.gate.wait_until_admitted(NATIVE_GATE_BOUND));
                assert!(matches!(
                    run.recv_timeout(Duration::ZERO),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
                panic!("injected assertion before gate release");
            });
            let panic = failure.expect_err("injected assertion must fail");
            assert_eq!(
                panic.downcast_ref::<&str>(),
                Some(&"injected assertion before gate release"),
                "cleanup must not replace the original assertion failure"
            );
            assert!(settled.load(Ordering::SeqCst));
            assert_eq!(
                dropped_by_receiver.load(Ordering::SeqCst),
                !panic_worker,
                "a returned result must reach the receiver, not become a SendError"
            );
        }
        let next = NativeRun::spawn(|| {
            test_hook::at_run_admission();
            7
        });
        assert!(next.gate.wait_until_admitted(NATIVE_GATE_BOUND));
        next.gate.release();
        assert_eq!(next.finish(), 7);
    }

    #[test]
    fn a_stuck_worker_is_contained_and_reaped_by_the_child_watchdog() {
        let test = "native_tests::lifecycle::tests::stuck_worker_fixture";
        let started = Instant::now();
        let output = child_output(test, Duration::from_secs(5));
        assert!(!output.within_bounds, "a stuck worker cannot pass");
        assert!(output.status.is_some(), "the terminated child is reaped");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("stuck worker admitted"),
            "the fixture must reach the worker before testing its containment"
        );
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn stuck_worker_fixture() {
        if std::env::var(CHILD_ENV).as_deref()
            != Ok("native_tests::lifecycle::tests::stuck_worker_fixture")
        {
            return;
        }
        let run: NativeRun<()> = NativeRun::spawn(|| {
            test_hook::at_run_admission();
            loop {
                thread::park();
            }
        });
        assert!(run.gate.wait_until_admitted(NATIVE_GATE_BOUND));
        eprintln!("stuck worker admitted");
        // Drop releases the gate and joins; only the parent can end this hang.
        drop(run);
    }
}
