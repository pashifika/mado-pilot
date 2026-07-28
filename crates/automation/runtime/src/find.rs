//! What a deep search asks for, and the envelope it answers with.
//!
//! A request names the frame to search as either the session's current
//! published frame or one exact frame the caller already holds. Those are
//! different questions and the difference is not inferable: "search what is on
//! screen now" and "search the frame I mapped a moment ago" produce different
//! answers as soon as a second frame is published, so a caller states which one
//! it means.
//!
//! The outcome adds what neither contract below it can know. The vision package
//! correlates a result with a frame identity but has never heard of a capture
//! session; the capture package publishes frames but does not know one was
//! searched. Naming the target, and carrying the exact frame that was searched,
//! is the engine's answer to "which frame is this result about" — one that stays
//! answerable after the session closes.

use mado_pilot_capture::{Frame, FrameView};
use mado_pilot_core::{ClipPolicy, Error, TargetId};
use mado_pilot_vision::{MatchOptions, MatchResult, PreparedTemplate, RegionSelection};

/// Which frame a search runs against.
///
/// This enum is `#[non_exhaustive]`: later phases add selections, and a caller
/// must keep a fallback arm.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum SearchFrame<'a> {
    /// The session's current published frame, acquired inside the search's own
    /// operation.
    Latest,
    /// This exact frame, which the caller already holds.
    Exact(&'a Frame),
}

/// One deep template search against one capture session.
///
/// Borrowed rather than owned, because a request is consumed by the call it was
/// built for and the outcome owns everything it reports.
#[derive(Debug)]
pub struct FindRequest<'a> {
    frame: SearchFrame<'a>,
    region: RegionSelection,
    template: &'a PreparedTemplate,
    options: MatchOptions,
}

impl<'a> FindRequest<'a> {
    /// Searches the whole of the session's current published frame.
    #[must_use]
    pub const fn latest(template: &'a PreparedTemplate, options: MatchOptions) -> Self {
        Self {
            frame: SearchFrame::Latest,
            region: RegionSelection::FullFrame,
            template,
            options,
        }
    }

    /// Searches the whole of one exact frame the caller holds.
    #[must_use]
    pub const fn exact(
        frame: &'a Frame,
        template: &'a PreparedTemplate,
        options: MatchOptions,
    ) -> Self {
        Self {
            frame: SearchFrame::Exact(frame),
            region: RegionSelection::FullFrame,
            template,
            options,
        }
    }

    /// Searches a view's region of the view's own exact source frame.
    ///
    /// The view has already resolved and validated its region, so this cannot
    /// reinterpret it: the region is carried through as discrete capture pixels
    /// under a rejecting policy.
    ///
    /// # Errors
    ///
    /// Returns a geometry error only when the view's own region is not
    /// representable as coordinate values, which a constructed view rules out.
    pub fn view(
        view: &'a FrameView,
        template: &'a PreparedTemplate,
        options: MatchOptions,
    ) -> Result<Self, Error> {
        Ok(Self {
            frame: SearchFrame::Exact(view.frame()),
            region: RegionSelection::pixels(view.region(), ClipPolicy::Reject)?,
            template,
            options,
        })
    }

    /// Narrows the search to `region` of the selected frame.
    #[must_use]
    pub const fn in_region(mut self, region: RegionSelection) -> Self {
        self.region = region;
        self
    }

    /// Returns which frame the search runs against.
    #[must_use]
    pub const fn frame(&self) -> SearchFrame<'a> {
        self.frame
    }

    /// Returns the region of that frame to search.
    #[must_use]
    pub const fn region(&self) -> RegionSelection {
        self.region
    }

    /// Returns the prepared template to search for.
    #[must_use]
    pub const fn template(&self) -> &'a PreparedTemplate {
        self.template
    }

    /// Returns the matching options.
    #[must_use]
    pub const fn options(&self) -> MatchOptions {
        self.options
    }
}

/// The authoritative outcome of one deep search.
///
/// It owns the exact frame that was searched, so "which frame is this result
/// about" stays answerable after the session has closed and after the caller
/// has released every value it used to ask. The cost is that the frame's pixels
/// live as long as the outcome does; [`FindOutcome::into_result`] releases them
/// while keeping the correlated result.
#[derive(Debug, Clone)]
pub struct FindOutcome {
    target: TargetId,
    frame: Frame,
    result: MatchResult,
}

impl FindOutcome {
    pub(crate) const fn new(target: TargetId, frame: Frame, result: MatchResult) -> Self {
        Self {
            target,
            frame,
            result,
        }
    }

    /// Returns the target whose session produced the searched frame.
    #[must_use]
    pub const fn target(&self) -> TargetId {
        self.target
    }

    /// Returns the exact frame that was searched.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns the immutable source-correlated matching result.
    #[must_use]
    pub const fn result(&self) -> &MatchResult {
        &self.result
    }

    /// Returns the result, releasing the searched frame.
    #[must_use]
    pub fn into_result(self) -> MatchResult {
        self.result
    }
}
