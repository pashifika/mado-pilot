//! What the capture contract suite does with an adapter that waits forever.
//!
//! `tests/capture_contract.rs` covers the adapter that answers wrongly. This
//! covers the one that does not answer: a provider that never publishes *and*
//! implements its own wait, so nothing it is given can end the call. That is
//! the one defect the suite cannot report by returning, because the thread that
//! would report it is the thread that is blocked.
//!
//! The suite ends the process for it, so the check runs in a child process and
//! this test reads what the child left behind. It costs the watchdog's bound —
//! twelve seconds — once per run, which is the price of proving that a hang is
//! diagnosed rather than waited out.

use std::fmt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, CoordinateSupport, Frame, FrameDescriptor,
    FrameRequest, Lifecycle, OpenRequest, PixelFormat, SessionDescription, StreamState,
    TargetDescription,
};
use mado_pilot_core::{
    IdentityIssuer, OperationContext, PixelExtent, ProviderId, Result, TargetId,
};
use mado_pilot_testkit::capture_contract;

/// Provider name qualifying this double's target identities.
const PROVIDER: ProviderId = ProviderId::new("waits-forever");

/// The test the parent runs as a child, by name.
const HANGING_TEST: &str = "the_suite_against_a_provider_that_never_returns";

/// Set for the child, so an ordinary `cargo test -- --ignored` cannot hang.
const CHILD: &str = "MADOPILOT_CAPTURE_CONTRACT_WATCHDOG_CHILD";

/// How long the parent waits for the child before calling the watchdog broken.
///
/// Five times the bound it is waiting on, so a loaded machine is slow rather
/// than failing.
const PATIENCE: Duration = Duration::from_secs(60);

/// How often the parent looks at the child.
const POLL: Duration = Duration::from_millis(100);

/// A provider whose sessions open, publish nothing, and wait on their own.
///
/// The defect is the pair: a session with no frame to serve is ordinary, and a
/// session that waits is ordinary, but one that waits on something it never
/// consults the operation context about cannot be stopped by the context. An
/// adapter delegating to [`StreamState`] never has this shape; one implementing
/// `CaptureSession::frame` itself can.
struct WaitsForever {
    issuer: Arc<IdentityIssuer>,
    target: TargetId,
    descriptor: FrameDescriptor,
}

impl WaitsForever {
    fn new() -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let target = issuer.issue_target(PROVIDER).expect("identity issued");

        Self {
            issuer,
            target,
            descriptor: FrameDescriptor::packed(PixelExtent::new(8, 6), PixelFormat::Rgba8)
                .expect("a valid descriptor"),
        }
    }
}

impl fmt::Debug for WaitsForever {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaitsForever")
            .field("engine", &self.issuer.engine())
            .finish()
    }
}

impl CaptureProvider for WaitsForever {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, _operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        Ok(vec![TargetDescription::new(
            self.target,
            "waits-forever",
            self.descriptor.extent(),
            self.descriptor.format(),
            CoordinateSupport::frame_only(),
        )])
    }

    fn open(
        &self,
        target: TargetId,
        _request: &OpenRequest,
        _operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        target.check_engine(self.issuer.engine())?;
        if target != self.target {
            return Err(CaptureFault::UnknownTarget.into());
        }

        let stream = self.issuer.issue_stream()?;
        Ok(Arc::new(Session {
            description: SessionDescription::new(
                target,
                stream,
                self.descriptor.extent(),
                self.descriptor.format(),
                CoordinateSupport::frame_only(),
            ),
            state: StreamState::new(stream),
        }) as Arc<dyn CaptureSession>)
    }
}

struct Session {
    description: SessionDescription,
    state: StreamState,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("stream", &self.description.stream())
            .finish()
    }
}

impl CaptureSession for Session {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, _request: &FrameRequest, _operation: &OperationContext) -> Result<Frame> {
        // The whole double: its own wait, reading neither the deadline nor the
        // cancellation token, for a publication that never comes.
        loop {
            std::thread::sleep(POLL);
        }
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}

#[test]
#[ignore = "never returns on purpose; the test below runs it as a child process"]
fn the_suite_against_a_provider_that_never_returns() {
    assert!(
        std::env::var_os(CHILD).is_some(),
        "this test blocks forever until its watchdog ends the process, so it \
         runs only as the child the bounded-failure test spawns"
    );

    capture_contract::run(&WaitsForever::new());
}

#[test]
fn a_provider_that_never_returns_fails_the_suite_in_bounded_time_naming_the_fault() {
    let mut child = Command::new(std::env::current_exe().expect("this test binary's own path"))
        .args(["--exact", HANGING_TEST, "--ignored", "--nocapture"])
        .env(CHILD, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the test binary runs itself");

    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("the child's state is readable") {
            Some(status) => break status,
            None if started.elapsed() < PATIENCE => std::thread::sleep(POLL),
            None => {
                let _ = child.kill();
                panic!(
                    "the contract suite did not end within {PATIENCE:?} against a \
                     provider that never returns: the watchdog is the only thing \
                     that can end that run, so it did not fire"
                );
            }
        }
    };
    let elapsed = started.elapsed();

    let output = child
        .wait_with_output()
        .expect("the child's output is readable");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(
        !status.success(),
        "a provider whose session never returns must fail the suite, but the \
         run reported success:\n{stderr}"
    );
    assert!(
        stderr.contains("did not honour the deadline"),
        "the failure must name the adapter's fault rather than read as a hung \
         test, but the run said:\n{stderr}"
    );
    assert!(
        stderr.contains("the_first_frame_is_the_start_of_the_stream"),
        "the failure must name the check that was waiting, which is the first \
         one that asks for a frame, but the run said:\n{stderr}"
    );
    assert!(
        elapsed < PATIENCE,
        "the run must end on the watchdog's bound rather than the patience of \
         whoever is watching it"
    );
}
