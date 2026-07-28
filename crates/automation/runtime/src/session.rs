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
use mado_pilot_core::{Operation, OperationContext, Result, StreamId, TargetId};
use mado_pilot_vision::{MatchRequest, Matcher};

use crate::find::{FindOutcome, FindRequest, SearchFrame};

/// An open capture session that can search its own frames.
///
/// Dropping a session does not close it. Close is explicit, because a caller
/// that still holds frames, mappings, or results has to be able to say when the
/// capture side is finished with them.
#[derive(Debug)]
pub struct Session {
    description: SessionDescription,
    capture: Arc<dyn CaptureSession>,
    matcher: Matcher,
}

impl Session {
    pub(crate) fn new(capture: Arc<dyn CaptureSession>, matcher: Matcher) -> Self {
        Self {
            // Read once: a session's accepted description cannot change, and a
            // result envelope that re-read it would be reporting the adapter's
            // current answer rather than the one the caller opened with.
            description: capture.description(),
            capture,
            matcher,
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
    /// A closed session starts no search, whichever frame the request names.
    /// Searching an exact frame the caller already holds needs nothing from the
    /// capture side and would otherwise succeed after close, but "this session
    /// is finished" is the session's answer to give, and a caller that has to
    /// know which frame it asked for to predict whether close is observed has
    /// been handed two contracts instead of one.
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

        if self.capture.is_closed() {
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

    /// Closes the session and drains in-flight frame waits.
    ///
    /// Idempotent. Frames, views, mappings, prepared templates, packages, and
    /// outcomes the caller already holds stay valid: they are owned by the
    /// caller, not by the session.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the
    /// deadline wins before the drain finishes. The session then stays closing,
    /// and a later close continues the lifecycle.
    pub fn close(&self, operation: &OperationContext) -> Result<()> {
        self.capture.close(operation)
    }

    /// Reports whether the session has finished closing.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.capture.is_closed()
    }
}
