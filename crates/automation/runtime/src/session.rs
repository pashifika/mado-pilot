//! One open capture session, bound to the engine's matching backend.
//!
//! Binding the two is what makes a search a single deep operation rather than a
//! sequence a caller has to get right. Acquiring a frame and searching it are
//! two contracts' operations, and between them sit the rules neither contract
//! owns: which frame a result is about, whether that frame is even this
//! session's, and whether the whole sequence still wins its race against the
//! deadline by the time an answer exists.

use std::sync::Arc;

use mado_pilot_capture::{CaptureFault, CaptureSession, Frame, FrameRequest, SessionDescription};
use mado_pilot_core::{InputCapability, Operation, OperationContext, Result, StreamId, TargetId};
use mado_pilot_input::{
    InputController, InputDescriptor, InputFault, InputReceipt, InputRequest, SequenceOutcome,
};
use mado_pilot_vision::{MatchRequest, Matcher};

use crate::find::{FindOutcome, FindRequest, SearchFrame};

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
}

impl Session {
    pub(crate) fn new(
        capture: Arc<dyn CaptureSession>,
        matcher: Matcher,
        input: Option<Arc<dyn InputController>>,
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
        self.capture.frame(request, operation)
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
        let mut attempt = Operation::admit(operation)?;

        if !self.capture.is_open() {
            return Err(CaptureFault::SessionClosed.into());
        }

        let frame = match request.frame() {
            SearchFrame::Latest => self.capture.frame(&FrameRequest::latest(), operation)?,
            SearchFrame::Exact(frame) => {
                // A frame from another stream would produce an envelope naming
                // this session's target for content it never published.
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

    /// Delivers one bounded sequence to this session's target.
    ///
    /// # One sequence at a time, and no queue
    ///
    /// The controller serializes its own sequences, so two callers cannot
    /// interleave a modifier and a key into a keystroke neither asked for. A
    /// sequence waits under the caller's own operation context and nothing
    /// accumulates behind it: one whose deadline passes while waiting delivers
    /// nothing and says so. Pressure is reported to callers rather than absorbed.
    ///
    /// # What the receipt is, and why a failure is usually not an error
    ///
    /// An operating system cannot recall a delivered event, so a sequence that
    /// stopped part-way answers with how far it got. Once anything may have
    /// reached the target, that account is this operation's terminal outcome and
    /// is returned even when the caller's deadline passed while it ran —
    /// replacing it with the interruption would discard the one fact the caller
    /// has to act on. A receipt that delivered nothing carries no such fact, so
    /// an operation that lost its race reports the interruption instead.
    ///
    /// Nothing here retries. A sequence that stopped part-way is not sent again
    /// through another mechanism, whatever the request permitted: the events
    /// already delivered cannot be taken back, and repeating them is not a
    /// recovery a caller could have asked for.
    ///
    /// # Errors
    ///
    /// Returns an unsupported outcome when this session established no input, an
    /// invalid-argument outcome for a request addressed to another target or
    /// carrying a source frame from another stream, a closed outcome once the
    /// session or the controller is closing, the input contract's own refusal
    /// for a request no permitted mechanism can satisfy, and the operation's
    /// terminal outcome when cancellation or the deadline wins with nothing
    /// delivered.
    pub fn send_input(
        &self,
        request: &InputRequest,
        operation: &OperationContext,
    ) -> Result<InputReceipt> {
        let mut attempt = Operation::admit(operation)?;

        let Some(controller) = self.input.as_ref() else {
            return Err(InputFault::DeliveryUnavailable.into());
        };
        if request.target() != self.description.target() {
            return Err(InputFault::ForeignTarget.into());
        }
        // A source frame from another stream would resolve this target's
        // coordinates against geometry it never published. Which frame identity
        // is this session's is the session's rule, exactly as it is for a search.
        if let Some(source) = request.pointer_geometry().source()
            && source.stream() != self.description.stream()
        {
            return Err(CaptureFault::ForeignStream.into());
        }
        // A session that has begun closing delivers nothing, whatever the
        // request asks for. Close means the caller is finished with this target.
        if !self.capture.is_open() {
            return Err(CaptureFault::SessionClosed.into());
        }
        attempt.checkpoint()?;

        let receipt = controller.execute(request, operation)?;
        if receipt.outcome() == SequenceOutcome::Unexecuted {
            // Nothing reached the target, so a late answer is still late and the
            // operation's own outcome is the truthful one.
            return Ok(attempt.commit(receipt)?);
        }
        Ok(receipt)
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
        let input = match self.input.as_ref() {
            Some(controller) => controller.close(operation),
            None => Ok(()),
        };
        let capture = self.capture.close(operation);
        input.and(capture)
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
        IdentityIssuer, MonotonicInstant, OperationContext, PixelExtent, Status,
    };
    use mado_pilot_testkit::{ControlledCapture, ControlledMatcher, ManualClock, match_fixtures};
    use mado_pilot_vision::{MatchBackend, MatchOptions, Matcher, PreparedTemplate};

    use crate::find::FindRequest;

    use super::Session;

    const EXTENT: PixelExtent = PixelExtent::new(32, 24);

    /// One session over controlled doubles, with the backend still reachable.
    struct Fixture {
        backend: Arc<ControlledMatcher>,
        matcher: Matcher,
        session: Session,
    }

    impl Fixture {
        fn new() -> Self {
            let issuer = Arc::new(IdentityIssuer::new());
            let capture = ControlledCapture::new(issuer, EXTENT, PixelFormat::Rgba8)
                .expect("a valid controlled provider");
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

            Self {
                backend,
                matcher: matcher.clone(),
                session: Session::new(opened, matcher, None),
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
}
