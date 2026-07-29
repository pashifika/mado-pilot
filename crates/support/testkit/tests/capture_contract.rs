//! The capture contract suite against an adapter that satisfies it and against
//! one that does not.
//!
//! `crates/adapter/replay/tests/capture_contract.rs` runs the suite against a
//! production adapter, which proves the checks pass. It cannot prove they can
//! fail, and a check that cannot fail is a comment.
//!
//! One double serves both directions here. Its sessions hold a frame published
//! when the session opened, so it is the provider [`capture_contract::run`]
//! documents — one that publishes without help — and the only thing that
//! varies is whether its `frame` honours the operation context or answers
//! straight from what it holds. Ignoring the context is the defect the deadline
//! checks exist to catch, and it is the one an adapter that implements its own
//! waiting rather than delegating to `StreamState` is most likely to have.

use std::fmt;
use std::sync::Arc;

use mado_pilot_capture::{
    CaptureFault, CaptureProvider, CaptureSession, Continuity, CoordinateSupport, Frame,
    FrameDescriptor, FrameRequest, Lifecycle, OpenRequest, PixelFormat, Publication,
    SessionDescription, StreamState, TargetDescription,
};
use mado_pilot_core::{
    IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, ProviderId, Result, TargetId,
};
use mado_pilot_testkit::capture_contract;

/// Provider name qualifying this double's target identities.
const PROVIDER: ProviderId = ProviderId::new("published-at-open");

/// Whether a session honours the operation context it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Discipline {
    /// Delegates to `StreamState`, which admits the request and commits its
    /// frame through the context.
    Honoured,
    /// Answers from the frame the session holds, reading nothing.
    Ignored,
}

/// A provider whose sessions have a frame ready the moment they open.
struct Double {
    issuer: Arc<IdentityIssuer>,
    target: TargetId,
    descriptor: FrameDescriptor,
    discipline: Discipline,
}

impl Double {
    fn new(discipline: Discipline) -> Self {
        let issuer = Arc::new(IdentityIssuer::new());
        let target = issuer.issue_target(PROVIDER).expect("identity issued");

        Self {
            issuer,
            target,
            descriptor: FrameDescriptor::packed(PixelExtent::new(8, 6), PixelFormat::Rgba8)
                .expect("a valid descriptor"),
            discipline,
        }
    }
}

impl fmt::Debug for Double {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Double")
            .field("engine", &self.issuer.engine())
            .field("discipline", &self.discipline)
            .finish()
    }
}

impl CaptureProvider for Double {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, _operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        Ok(vec![TargetDescription::new(
            self.target,
            "published-at-open",
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
        let state = StreamState::new(stream);
        // Published at open, so no check ever waits for this double and the
        // only thing a session can get wrong is when it refuses to answer.
        state.publish(Publication {
            captured_at: MonotonicInstant::ORIGIN,
            descriptor: self.descriptor,
            placement: None,
            continuity: Continuity::Continuous,
            pixels: vec![0x77; self.descriptor.byte_len()].into_boxed_slice(),
        })?;

        Ok(Arc::new(Session {
            description: SessionDescription::new(
                target,
                stream,
                self.descriptor.extent(),
                self.descriptor.format(),
                CoordinateSupport::frame_only(),
            ),
            state,
            discipline: self.discipline,
        }) as Arc<dyn CaptureSession>)
    }
}

struct Session {
    description: SessionDescription,
    state: StreamState,
    discipline: Discipline,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("stream", &self.description.stream())
            .field("discipline", &self.discipline)
            .finish()
    }
}

impl CaptureSession for Session {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        match self.discipline {
            Discipline::Honoured => self.state.frame(request, operation),
            Discipline::Ignored => self
                .state
                .current()
                .ok_or_else(|| CaptureFault::SessionClosed.into()),
        }
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}

/// Runs `check` against a context-ignoring double and reports whether it was
/// rejected.
///
/// A contract check reports a violation by panicking, so catching the unwind is
/// how a test asserts that it did. The panic message reaches stderr on the way
/// through, which is what a reader wants to see when this test itself fails.
fn rejects_an_adapter_that_reads_nothing(check: fn(&dyn CaptureProvider)) -> bool {
    let provider = Double::new(Discipline::Ignored);

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&provider))).is_err()
}

#[test]
fn a_provider_that_publishes_at_open_satisfies_the_whole_suite() {
    capture_contract::run(&Double::new(Discipline::Honoured));
}

#[test]
fn an_adapter_that_serves_a_frame_past_its_deadline_is_rejected() {
    assert!(
        rejects_an_adapter_that_reads_nothing(
            capture_contract::an_already_expired_request_is_refused
        ),
        "a session that answers a request whose deadline has already passed \
         must fail the contract suite"
    );
}

#[test]
fn an_adapter_that_never_consults_the_operation_context_is_rejected() {
    assert!(
        rejects_an_adapter_that_reads_nothing(
            capture_contract::no_deadline_inside_a_frame_request_produces_a_frame
        ),
        "a session that reads neither the deadline nor the clock must fail the \
         contract suite, whatever it returns"
    );
}
