//! One open capture session, bound to the engine's matching backend.
//!
//! Binding the two is what makes a search a single deep operation rather than a
//! sequence a caller has to get right. Acquiring a frame and searching it are
//! two contracts' operations, and between them sit the rules neither contract
//! owns: which frame a result is about, whether that frame is even this
//! session's, and whether the whole sequence still wins its race against the
//! deadline by the time an answer exists.

use std::sync::Arc;

use mado_pilot_capture::{
    CaptureFault, CaptureSession, CpuMapping, Frame, FrameRequest, FrameView, PixelFormat,
    SessionDescription,
};
use mado_pilot_core::{
    ClipPolicy, CoordinateSpace, InputCapability, Lifecycle, Operation, OperationContext, Rect,
    Result, Status, StreamId, TargetId,
};
use mado_pilot_input::{
    InputController, InputDescriptor, InputFault, InputReceipt, InputRequest, SequenceOutcome,
};
use mado_pilot_vision::{MatchRequest, Matcher};

use crate::diagnostic::{
    DetachedObservation, DiagnosticEmitter, DiagnosticOperationKind, DiagnosticPayload,
    DiagnosticSink, FrameDiagnostic, InputDiagnostic, LifecycleDiagnostic, MappingDiagnostic,
    ObservedOperation, RouteAttemptDiagnostic, SearchDiagnostic, SearchDiagnosticOutcome,
};
use crate::find::{FindOutcome, FindRequest, SearchFrame};

/// A non-owning diagnostic projection copied into retained frame handles.
///
/// This exists for foreign ownership boundaries whose frame can outlive its
/// session. It retains only public identities and a weak diagnostic stream
/// reference, so keeping a frame never keeps an engine or session open.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct MappingObserver {
    target: TargetId,
    stream: StreamId,
    diagnostics: Option<DiagnosticEmitter>,
}

impl MappingObserver {
    fn new(target: TargetId, stream: StreamId, diagnostics: Option<DiagnosticEmitter>) -> Self {
        Self {
            target,
            stream,
            diagnostics,
        }
    }

    fn observe(&self, operation: &OperationContext) -> Result<Option<DetachedObservation>> {
        self.diagnostics
            .as_ref()
            .map(|diagnostics| diagnostics.observe(operation, DiagnosticOperationKind::Mapping))
            .transpose()
            .map(Option::flatten)
    }

    fn map(
        &self,
        frame: &Frame,
        source: CoordinateSpace,
        operation: &OperationContext,
        map: impl FnOnce() -> Result<CpuMapping>,
    ) -> Result<CpuMapping> {
        let observed = self.observe(operation)?;
        if frame.stamp().stream() != self.stream {
            return Err(CaptureFault::ForeignStream.into());
        }

        let result = map();
        if let (Some(observed), Ok(mapping)) = (&observed, &result) {
            observed.debug(operation, || {
                DiagnosticPayload::Mapping(MappingDiagnostic {
                    target: self.target,
                    frame: mapping.stamp(),
                    source,
                    destination: CoordinateSpace::CapturePixels,
                })
            });
        }
        result
    }

    /// Maps the whole retained frame while preserving runtime diagnostics.
    pub fn map_frame(
        &self,
        frame: &Frame,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> Result<CpuMapping> {
        self.map(frame, CoordinateSpace::CapturePixels, operation, || {
            frame.map(format, operation)
        })
    }

    /// Resolves and maps one retained frame region while preserving diagnostics.
    pub fn map_region(
        &self,
        frame: &Frame,
        region: Rect,
        policy: ClipPolicy,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> Result<CpuMapping> {
        self.map(frame, region.space(), operation, || {
            frame.view(region, policy)?.map(format, operation)
        })
    }

    /// Maps a validated retained frame view while preserving diagnostics.
    pub fn map_view(
        &self,
        view: &FrameView,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> Result<CpuMapping> {
        self.map(
            view.frame(),
            CoordinateSpace::CapturePixels,
            operation,
            || view.map(format, operation),
        )
    }
}

/// An open capture session that can search its own frames, and deliver input to
/// the target it captures when the open established any.
///
/// Dropping a session does not close it. Close is explicit, because a caller
/// that still holds frames, mappings, or results has to be able to say when the
/// capture side is finished with them.
#[derive(Debug)]
pub struct Session {
    description: SessionDescription,
    capture: Arc<dyn CaptureSession>,
    matcher: Matcher,
    input: Option<Arc<dyn InputController>>,
    input_descriptor: InputDescriptor,
    diagnostics: Option<DiagnosticSink>,
}

impl Session {
    pub(crate) fn new(
        capture: Arc<dyn CaptureSession>,
        matcher: Matcher,
        input: Option<Arc<dyn InputController>>,
        diagnostics: Option<DiagnosticSink>,
    ) -> Self {
        // Read once, for the reason the capture description is: an accepted
        // descriptor cannot change, and re-reading it would report the adapter's
        // current answer rather than the one the caller opened with.
        let description = capture.description();
        let input_descriptor = match input.as_ref() {
            Some(controller) => controller.descriptor(),
            // Truthful rather than absent: through this session the target
            // accepts no input, which is what a caller has to be able to read
            // without first knowing how the engine was wired.
            None => InputDescriptor::new(description.target(), InputCapability::none()),
        };

        Self {
            description,
            capture,
            matcher,
            input,
            input_descriptor,
            diagnostics,
        }
    }

    fn observe(
        &self,
        operation: &OperationContext,
        kind: DiagnosticOperationKind,
    ) -> Result<Option<ObservedOperation>> {
        self.diagnostics
            .as_ref()
            .map(|diagnostics| diagnostics.observe(operation, kind))
            .transpose()
    }

    fn normal(
        &self,
        observed: Option<ObservedOperation>,
        operation: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if let (Some(diagnostics), Some(observed)) = (&self.diagnostics, observed) {
            diagnostics.normal(observed, operation, payload);
        }
    }

    fn debug(
        &self,
        observed: Option<ObservedOperation>,
        operation: &OperationContext,
        payload: impl FnOnce() -> DiagnosticPayload,
    ) {
        if let (Some(diagnostics), Some(observed)) = (&self.diagnostics, observed) {
            diagnostics.debug(observed, operation, payload);
        }
    }

    /// Returns what this session actually accepted.
    #[must_use]
    pub const fn description(&self) -> &SessionDescription {
        &self.description
    }

    /// Returns the target this session captures.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.description.target()
    }

    /// Returns the stream this session publishes to.
    #[must_use]
    pub const fn stream(&self) -> StreamId {
        self.description.stream()
    }

    /// Returns the frame `request` asks for, waiting when necessary.
    ///
    /// A verb, because this can block: asking for the latest frame waits until
    /// one exists. The accessors on this type are nouns and none of them waits
    /// for anything.
    ///
    /// Frame identity, ordering, and latest-frame semantics are the capture
    /// package's; this hands the request to the adapter unchanged.
    ///
    /// # Errors
    ///
    /// Returns a closed outcome once the session is closing, an
    /// invalid-argument outcome for a stamp from another stream, and the
    /// operation's terminal outcome when cancellation or the deadline wins.
    pub fn acquire_frame(
        &self,
        request: &FrameRequest,
        operation: &OperationContext,
    ) -> Result<Frame> {
        let observed = self.observe(operation, DiagnosticOperationKind::FrameAcquire)?;
        let result = self.capture.frame(request, operation);
        match &result {
            Ok(frame) => {
                self.debug(observed, operation, || {
                    DiagnosticPayload::Frame(FrameDiagnostic {
                        target: self.target(),
                        frame: frame.stamp(),
                    })
                });
            }
            Err(error) if error.status() == Status::TargetLost => {
                self.normal(observed, operation, || {
                    DiagnosticPayload::Lifecycle(LifecycleDiagnostic {
                        target: Some(self.target()),
                        lifecycle: Lifecycle::Closed,
                        fault: Some(Status::TargetLost),
                    })
                });
            }
            Err(_) => {}
        }
        result
    }

    /// Maps the whole frame and emits a debug mapping fact when enabled.
    ///
    /// The frame must belong to this session's stream. Mapping remains valid
    /// after the session closes because it reads retained immutable frame state.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a frame from another stream, the
    /// mapping fault for an unsupported descriptor, or the operation's terminal
    /// deadline or cancellation outcome.
    pub fn map_frame(
        &self,
        frame: &Frame,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> Result<CpuMapping> {
        self.mapping_observer().map_frame(frame, format, operation)
    }

    /// Resolves and maps one frame region and emits a debug mapping fact.
    ///
    /// # Errors
    ///
    /// In addition to [`Session::map_frame`] errors, returns a coordinate or
    /// bounds fault when `region` cannot be resolved under `policy`.
    pub fn map_region(
        &self,
        frame: &Frame,
        region: Rect,
        policy: ClipPolicy,
        format: PixelFormat,
        operation: &OperationContext,
    ) -> Result<CpuMapping> {
        self.mapping_observer()
            .map_region(frame, region, policy, format, operation)
    }

    /// Copies the non-owning observer used by retained foreign frame handles.
    #[doc(hidden)]
    #[must_use]
    pub fn mapping_observer(&self) -> MappingObserver {
        MappingObserver::new(
            self.target(),
            self.stream(),
            self.diagnostics.as_ref().map(DiagnosticSink::emitter),
        )
    }

    /// Searches one of this session's frames for one prepared template.
    ///
    /// The whole sequence runs under one operation: the frame is acquired, the
    /// backend runs, and only then is the envelope committed. A deadline that
    /// passes after the matcher produced a perfectly good result and before the
    /// envelope exists therefore reports deadline expiry, and the late work
    /// never becomes observable.
    ///
    /// The capture adapter and the matcher each arbitrate their own terminal
    /// outcome too, so in practice one of them usually observes an interruption
    /// first. That is the intent rather than a redundancy: this operation is
    /// what makes the deep search *one* operation with one terminal outcome,
    /// instead of a sequence that is correct only because each of its steps
    /// happened to check.
    ///
    /// A session that has begun closing starts no search, whichever frame the
    /// request names. Searching an exact frame the caller already holds needs
    /// nothing from the capture side and would otherwise succeed after close,
    /// but "this session is finished" is the session's answer to give, and a
    /// caller that has to know which frame it asked for to predict whether close
    /// is observed has been handed two contracts instead of one. The gate is
    /// "close has begun" rather than "close has finished", because a close whose
    /// operation is cancelled or already expired leaves the session closing, and
    /// a latest-frame search is already refused in that state.
    ///
    /// # Errors
    ///
    /// Returns a closed outcome once the session is closing, an
    /// invalid-argument outcome for a frame published by another stream, the
    /// capture failure for an acquisition that could not be satisfied, the
    /// vision failure for a search that could not run, and the operation's
    /// terminal outcome when cancellation or the deadline wins.
    pub fn find_template(
        &self,
        request: &FindRequest<'_>,
        operation: &OperationContext,
    ) -> Result<FindOutcome> {
        let observed = self.observe(operation, DiagnosticOperationKind::Search)?;
        let template = self
            .diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.template(request.template()));
        let requested_frame = match request.frame() {
            SearchFrame::Latest => None,
            SearchFrame::Exact(frame) => Some(frame.stamp()),
        };

        let result: Result<FindOutcome> = (|| {
            let mut attempt = Operation::admit(operation)?;
            if !self.capture.is_open() {
                return Err(CaptureFault::SessionClosed.into());
            }

            let frame = match request.frame() {
                SearchFrame::Latest => self.capture.frame(&FrameRequest::latest(), operation)?,
                SearchFrame::Exact(frame) => {
                    if frame.stamp().stream() != self.description.stream() {
                        return Err(CaptureFault::ForeignStream.into());
                    }
                    frame.clone()
                }
            };
            attempt.checkpoint()?;
            let result = self.matcher.find(
                MatchRequest::new(
                    &frame,
                    request.region(),
                    request.template(),
                    request.options(),
                ),
                operation,
            )?;
            let outcome = FindOutcome::new(self.description.target(), frame, result);
            Ok(attempt.commit(outcome)?)
        })();

        if let Some(template) = template {
            let (frame, region, outcome, result_count) = match &result {
                Ok(outcome) => (
                    Some(outcome.frame().stamp()),
                    Some(outcome.result().searched()),
                    if outcome.result().is_empty() {
                        SearchDiagnosticOutcome::NoMatch
                    } else {
                        SearchDiagnosticOutcome::Matched
                    },
                    outcome.result().matches().len() as u64,
                ),
                Err(error) => (
                    requested_frame,
                    None,
                    SearchDiagnosticOutcome::Failed(error.status()),
                    0,
                ),
            };
            self.normal(observed, operation, || {
                DiagnosticPayload::Search(SearchDiagnostic {
                    target: self.target(),
                    frame,
                    template,
                    region,
                    outcome,
                    result_count,
                })
            });
        } else if let Some(diagnostics) = &self.diagnostics {
            diagnostics.normal_loss();
        }
        result
    }

    /// Returns what input this session actually established.
    ///
    /// Always answers. A session opened without input, and one whose optional
    /// input capability turned out to be unavailable, both report a descriptor
    /// that accepts nothing, so a caller reads what it got rather than inferring
    /// it from how the engine was wired.
    #[must_use]
    pub const fn input_descriptor(&self) -> &InputDescriptor {
        &self.input_descriptor
    }

    /// Reports whether this session can deliver input to its target.
    #[must_use]
    pub fn accepts_input(&self) -> bool {
        self.input.is_some() && self.input_descriptor.is_available()
    }

    /// Submits one bounded sequence to this session's target.
    ///
    /// # One sequence at a time, and no queue
    ///
    /// The controller serializes its own sequences, so two callers cannot
    /// interleave a modifier and a key into a keystroke neither asked for. A
    /// sequence waits under the caller's own operation context and nothing
    /// accumulates behind it: one whose deadline passes while waiting reaches no
    /// route and says so. Pressure is reported to callers rather than absorbed.
    ///
    /// # What the receipt is, and why a failure is usually not an error
    ///
    /// An operating system cannot recall an event that may already have native
    /// effect, so a sequence that stopped part-way answers with how far its route
    /// got. Once anything may have reached the target, that account is this
    /// operation's terminal outcome and is returned even when the caller's
    /// deadline passed while it ran — replacing it with the interruption would
    /// discard the one fact the caller has to act on. A receipt with no possible
    /// native effect carries no such fact, so an operation that lost its race
    /// reports the interruption instead.
    ///
    /// Nothing here retries. A sequence that stopped part-way is not sent again
    /// through another mechanism, whatever the request permitted: possible native
    /// effect cannot be taken back, and repeating it is not a recovery a caller
    /// could have asked for.
    ///
    /// # Errors
    ///
    /// Returns an unsupported outcome when this session established no input, an
    /// invalid-argument outcome for a request addressed to another target or
    /// carrying a source frame from another stream, a closed outcome once the
    /// session or the controller is closing, the input contract's own refusal
    /// for a request no permitted mechanism can satisfy, and the operation's
    /// terminal outcome when cancellation or the deadline wins before any
    /// possible native effect.
    pub fn send_input(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<InputReceipt> {
        let observed = self.observe(operation, DiagnosticOperationKind::InputSubmission)?;
        let result: Result<InputReceipt> = (|| {
            let mut attempt = Operation::admit(operation)?;
            let Some(controller) = self.input.as_ref() else {
                return Err(InputFault::RouteUnavailable.into());
            };
            if request.target() != self.description.target() {
                return Err(InputFault::ForeignTarget.into());
            }
            if let Some(source) = request.pointer_geometry().source()
                && source.stream() != self.description.stream()
            {
                return Err(CaptureFault::ForeignStream.into());
            }
            if !self.capture.is_open() {
                return Err(CaptureFault::SessionClosed.into());
            }
            attempt.checkpoint()?;

            let receipt = controller.execute(request, operation)?;
            if receipt.outcome() == SequenceOutcome::Unexecuted {
                return Ok(attempt.commit(receipt)?);
            }
            Ok(receipt)
        })();

        if let Ok(receipt) = &result {
            for attempt in receipt.attempts() {
                let attempt = *attempt;
                self.debug(observed, operation, || {
                    DiagnosticPayload::RouteAttempt(RouteAttemptDiagnostic {
                        target: receipt.target(),
                        route: attempt.route(),
                        address_scope: attempt.address_scope(),
                        evidence: attempt.evidence(),
                        outcome: attempt.outcome(),
                        submitted: attempt.submitted() as u64,
                        partial_native_effect: attempt.partial_native_effect(),
                        fault: attempt.fault(),
                    })
                });
            }
        }
        self.normal(observed, operation, || {
            DiagnosticPayload::Input(match &result {
                Ok(receipt) => InputDiagnostic::from_receipt(request, receipt),
                Err(error) => InputDiagnostic::from_failure(request, error.status()),
            })
        });
        result
    }

    /// Closes the session and drains in-flight frame waits and input sequences.
    ///
    /// Idempotent, and retryable: a close that loses its own race leaves both
    /// sides closing, and a later close continues the drain rather than
    /// restarting it or repeating an event. Input stops first, because it is the
    /// side that can still change the target.
    ///
    /// Frames, views, mappings, prepared templates, packages, outcomes, and
    /// receipts the caller already holds stay valid: they are owned by the
    /// caller, not by the session.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins before either drain finishes. Both drains are attempted
    /// whatever the first one reports, so a retried close continues one lifecycle
    /// rather than two that diverged.
    pub fn close(&self, operation: &OperationContext) -> Result<()> {
        let observed = self.observe(operation, DiagnosticOperationKind::SessionClose)?;
        let input = match self.input.as_ref() {
            Some(controller) => controller.close(operation),
            None => Ok(()),
        };
        let capture = self.capture.close(operation);
        let result = input.and(capture);
        self.normal(observed, operation, || {
            DiagnosticPayload::Lifecycle(LifecycleDiagnostic {
                target: Some(self.target()),
                lifecycle: if self.is_closed() {
                    Lifecycle::Closed
                } else {
                    Lifecycle::Closing
                },
                fault: result.as_ref().err().map(|error| error.status()),
            })
        });
        result
    }

    /// Reports whether the session has finished closing.
    ///
    /// A session that has begun closing but not finished draining reports
    /// `false` here and still refuses work: this answers "is the lifecycle over",
    /// which is what the C boundary's `session_is_closed` reports, and not "will
    /// this session accept a request".
    ///
    /// Both sides have to be over. A capture side that finished draining while a
    /// sequence is still unwinding is a session whose target can still change.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.capture.is_closed()
            && self
                .input
                .as_ref()
                .is_none_or(|controller| controller.is_closed())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mado_pilot_capture::{CaptureProvider, Continuity, FrameRequest, OpenRequest, PixelFormat};
    use mado_pilot_core::{
        ClipPolicy, IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, PixelRect,
        Status,
    };
    use mado_pilot_testkit::{ControlledCapture, ControlledMatcher, ManualClock, match_fixtures};
    use mado_pilot_vision::{
        MatchBackend, MatchOptions, Matcher, PreparedTemplate, RegionSelection,
    };

    use crate::diagnostic::{
        DiagnosticBatch, DiagnosticDrain, DiagnosticLevel, DiagnosticOptions, DiagnosticPayload,
        DiagnosticReader, DiagnosticSink, SearchDiagnosticOutcome,
    };
    use crate::find::FindRequest;

    use super::Session;

    const EXTENT: PixelExtent = PixelExtent::new(32, 24);

    /// One session over controlled doubles, with the backend still reachable.
    struct Fixture {
        capture: Arc<ControlledCapture>,
        backend: Arc<ControlledMatcher>,
        matcher: Matcher,
        session: Session,
        reader: Option<DiagnosticReader>,
    }

    impl Fixture {
        fn new() -> Self {
            Self::build(false)
        }

        fn with_diagnostics() -> Self {
            Self::build(true)
        }

        fn build(with_diagnostics: bool) -> Self {
            let issuer = Arc::new(IdentityIssuer::new());
            let capture = Arc::new(
                ControlledCapture::new(issuer, EXTENT, PixelFormat::Rgba8)
                    .expect("a valid controlled provider"),
            );
            let backend = Arc::new(ControlledMatcher::new(PixelFormat::Rgba8));
            let matcher = Matcher::new(Arc::clone(&backend) as Arc<dyn MatchBackend>);
            let operation = OperationContext::new();
            let target = capture.discover(&operation).expect("discovered").remove(0);
            let opened = capture
                .open(target.id(), &OpenRequest::new(), &operation)
                .expect("opened");
            capture
                .publish(0x11, Continuity::Continuous)
                .expect("published");
            let (diagnostics, reader) = if with_diagnostics {
                let (sink, reader) = DiagnosticSink::create(
                    DiagnosticOptions::normal(8).expect("valid diagnostic capacity"),
                )
                .expect("enabled diagnostics");
                (Some(sink), Some(reader))
            } else {
                (None, None)
            };

            Self {
                capture,
                backend,
                matcher: matcher.clone(),
                session: Session::new(opened, matcher, None, diagnostics),
                reader,
            }
        }

        fn template(&self) -> PreparedTemplate {
            self.matcher
                .prepare(
                    &match_fixtures::planted_template("patch"),
                    &OperationContext::new(),
                )
                .expect("prepared")
        }

        fn diagnostic_batch(&self) -> DiagnosticBatch {
            match self.reader.as_ref().expect("enabled reader").drain() {
                DiagnosticDrain::Batch(batch) => batch,
                other => panic!("expected diagnostic batch, got {other:?}"),
            }
        }
    }

    /// An operation whose deadline has already passed on its own clock.
    fn expired() -> OperationContext {
        OperationContext::new()
            .with_clock(Arc::new(ManualClock::new()))
            .with_deadline(MonotonicInstant::ORIGIN)
    }

    #[test]
    fn a_session_that_has_begun_closing_starts_no_search() {
        let fixture = Fixture::new();
        let operation = OperationContext::new();
        let template = fixture.template();
        let options = MatchOptions::from_defaults(template.defaults());
        let frame = fixture
            .session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect("a published frame");

        // Close begins before its operation is admitted, so a close that loses
        // its own race leaves the session closing rather than closed.
        let close = fixture
            .session
            .close(&expired())
            .expect_err("the deadline wins the close");

        assert_eq!(close.status(), Status::DeadlineExceeded);
        assert!(
            !fixture.session.is_closed(),
            "the drain never finished, so the lifecycle is not over"
        );

        let exact = fixture
            .session
            .find_template(&FindRequest::exact(&frame, &template, options), &operation)
            .expect_err("a closing session starts no search");
        let latest = fixture
            .session
            .find_template(&FindRequest::latest(&template, options), &operation)
            .expect_err("a closing session starts no search");

        assert_eq!(exact.status(), Status::Closed);
        assert_eq!(
            latest.status(),
            Status::Closed,
            "a caller must not have to know which frame it asked for"
        );
        assert_eq!(fixture.backend.find_count(), 0);
    }

    #[test]
    fn an_open_session_searches_an_exact_frame_the_caller_holds() {
        let fixture = Fixture::new();
        let operation = OperationContext::new();
        let template = fixture.template();
        let options = MatchOptions::from_defaults(template.defaults());
        let frame = fixture
            .session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect("a published frame");

        let outcome = fixture
            .session
            .find_template(&FindRequest::exact(&frame, &template, options), &operation)
            .expect("an open session searches");

        assert_eq!(outcome.frame().stamp(), frame.stamp());
        assert_eq!(fixture.backend.find_count(), 1);
    }

    #[test]
    fn successful_search_diagnostics_report_each_effective_clipped_region() {
        let fixture = Fixture::with_diagnostics();
        let operation = OperationContext::new();
        let template = fixture.template();
        let options = MatchOptions::from_defaults(template.defaults());
        let frame = fixture
            .session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect("published frame");
        let first_region = PixelRect::new(1, 2, 20, 20).expect("valid");
        let clipped_request = PixelRect::new(16, 12, 40, 32).expect("valid");
        let clipped_region = PixelRect::new(16, 12, 32, 24).expect("valid");

        let first = fixture
            .session
            .find_template(
                &FindRequest::exact(&frame, &template, options).in_region(
                    RegionSelection::pixels(first_region, ClipPolicy::Reject)
                        .expect("valid selection"),
                ),
                &operation,
            )
            .expect("first successful search");
        let second = fixture
            .session
            .find_template(
                &FindRequest::exact(&frame, &template, options).in_region(
                    RegionSelection::pixels(clipped_request, ClipPolicy::Clip)
                        .expect("valid selection"),
                ),
                &operation,
            )
            .expect("second successful search");

        assert_eq!(first.result().searched(), first_region);
        assert_eq!(second.result().searched(), clipped_region);
        let batch = fixture.diagnostic_batch();
        assert!(batch.losses().is_empty());
        let searches: Vec<_> = batch
            .records()
            .iter()
            .filter_map(|record| match record.payload() {
                DiagnosticPayload::Search(search) => Some(search),
                _ => None,
            })
            .collect();
        assert_eq!(searches.len(), 2);
        assert_eq!(searches[0].region, Some(first_region));
        assert_eq!(searches[1].region, Some(clipped_region));
    }

    #[test]
    fn failed_search_diagnostic_has_no_completed_region() {
        let fixture = Fixture::with_diagnostics();
        let operation = OperationContext::new();
        let template = fixture.template();
        let requested = PixelRect::new(1, 2, 20, 20).expect("valid");
        fixture.session.close(&operation).expect("closed");
        let _ = fixture.diagnostic_batch();

        let error = fixture
            .session
            .find_template(
                &FindRequest::latest(&template, MatchOptions::from_defaults(template.defaults()))
                    .in_region(
                        RegionSelection::pixels(requested, ClipPolicy::Reject)
                            .expect("valid selection"),
                    ),
                &operation,
            )
            .expect_err("a closed session cannot search");
        assert_eq!(error.status(), Status::Closed);

        let batch = fixture.diagnostic_batch();
        assert_eq!(batch.records().len(), 1);
        assert!(matches!(
            batch.records()[0].payload(),
            DiagnosticPayload::Search(search)
                if search.region.is_none()
                    && search.outcome == SearchDiagnosticOutcome::Failed(Status::Closed)
        ));
    }

    #[test]
    fn direct_target_loss_emits_one_normal_closed_lifecycle_record() {
        let fixture = Fixture::with_diagnostics();
        let operation = OperationContext::new();
        let target = fixture.session.target();
        fixture.capture.lose(target);

        let error = fixture
            .session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect_err("the adapter reports its lost target");
        assert_eq!(error.status(), Status::TargetLost);

        let batch = fixture.diagnostic_batch();
        assert!(batch.losses().is_empty());
        assert_eq!(batch.records().len(), 1);
        let record = batch.records()[0];
        assert_eq!(record.level(), DiagnosticLevel::Normal);
        assert!(matches!(
            record.payload(),
            DiagnosticPayload::Lifecycle(lifecycle)
                if lifecycle.target == Some(target)
                    && lifecycle.lifecycle == mado_pilot_core::Lifecycle::Closed
                    && lifecycle.fault == Some(Status::TargetLost)
        ));
    }
}
