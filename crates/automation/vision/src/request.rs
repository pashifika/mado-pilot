//! What a caller asks for, stated so that nothing is left implied.
//!
//! A request names the exact frame, the region of it to search, the prepared
//! template, and the options. Every one of those is explicit: there is no
//! "current frame", no implied full-frame default when a region fails to
//! resolve, and no threshold that appears from somewhere the caller cannot see.

use mado_pilot_capture::{Frame, FrameView};
use mado_pilot_core::{ClipPolicy, CoordinateSpace, PixelRect, Rect};

use crate::fault::VisionFault;
use crate::prepared::PreparedTemplate;
use crate::template::MatchDefaults;

/// Which part of the source frame to search.
///
/// This enum is `#[non_exhaustive]`: later phases add selections, and a caller
/// must keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum RegionSelection {
    /// The frame's complete half-open capture-pixel extent.
    FullFrame,
    /// A region in a named coordinate space, resolved against the exact source
    /// frame's own transform snapshot.
    Region {
        /// The requested region.
        rect: Rect,
        /// What to do when it does not fit the frame.
        policy: ClipPolicy,
    },
}

impl RegionSelection {
    /// Selects an already-discrete capture-pixel region.
    ///
    /// # Errors
    ///
    /// Returns a geometry fault when an edge is not representable as a
    /// coordinate value.
    pub fn pixels(region: PixelRect, policy: ClipPolicy) -> Result<Self, mado_pilot_core::Error> {
        let rect = Rect::new(
            CoordinateSpace::CapturePixels,
            f64::from(region.left()),
            f64::from(region.top()),
            f64::from(region.right()),
            f64::from(region.bottom()),
        )?;
        Ok(RegionSelection::Region { rect, policy })
    }
}

/// What to do when two surviving candidates overlap.
///
/// There is deliberately no overlap ratio to tune. Two reported matches of one
/// template that sit on top of each other are one match reported twice, and a
/// ratio would turn that into a number nobody has evidence for. A ratio variant
/// can be added additively if a fixture ever shows one is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Suppression {
    /// Drop a candidate that overlaps a canonically earlier surviving one.
    #[default]
    DropOverlapping,
    /// Report every candidate that passed the threshold.
    KeepAll,
}

/// The validated options one search runs under.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchOptions {
    min_score: f64,
    max_results: u32,
    suppression: Suppression,
}

impl MatchOptions {
    /// Builds options from a template's authored defaults.
    #[must_use]
    pub const fn from_defaults(defaults: MatchDefaults) -> Self {
        Self {
            min_score: defaults.min_score(),
            max_results: defaults.max_results(),
            suppression: Suppression::DropOverlapping,
        }
    }

    /// Overrides the minimum score a match must reach.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::InvalidMatchScore`] when `min_score` is not a
    /// finite value inside `0.0..=1.0`.
    pub fn with_min_score(mut self, min_score: f64) -> Result<Self, VisionFault> {
        if !min_score.is_finite() || !(0.0..=1.0).contains(&min_score) {
            return Err(VisionFault::InvalidMatchScore);
        }
        self.min_score = min_score;
        Ok(self)
    }

    /// Overrides the largest number of matches to report.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::InvalidMatchResultLimit`] when `max_results` is
    /// zero, which asks for no results at all.
    pub const fn with_max_results(mut self, max_results: u32) -> Result<Self, VisionFault> {
        if max_results == 0 {
            return Err(VisionFault::InvalidMatchResultLimit);
        }
        self.max_results = max_results;
        Ok(self)
    }

    /// Overrides the overlap policy.
    #[must_use]
    pub const fn with_suppression(mut self, suppression: Suppression) -> Self {
        self.suppression = suppression;
        self
    }

    /// Returns the minimum score a match must reach.
    #[must_use]
    pub const fn min_score(self) -> f64 {
        self.min_score
    }

    /// Returns the largest number of matches to report.
    #[must_use]
    pub const fn max_results(self) -> u32 {
        self.max_results
    }

    /// Returns the overlap policy.
    #[must_use]
    pub const fn suppression(self) -> Suppression {
        self.suppression
    }
}

/// One template-matching request against one exact frame.
///
/// The frame is borrowed rather than cloned, because a request is consumed by
/// the call it is built for. The result it produces owns everything it reports,
/// so nothing here has to outlive the search.
#[derive(Debug)]
pub struct MatchRequest<'a> {
    frame: &'a Frame,
    selection: RegionSelection,
    template: &'a PreparedTemplate,
    options: MatchOptions,
}

impl<'a> MatchRequest<'a> {
    /// Builds a request against `frame`.
    #[must_use]
    pub const fn new(
        frame: &'a Frame,
        selection: RegionSelection,
        template: &'a PreparedTemplate,
        options: MatchOptions,
    ) -> Self {
        Self {
            frame,
            selection,
            template,
            options,
        }
    }

    /// Builds a request against a view's exact source frame and its region.
    ///
    /// A view has already resolved and validated its region against the frame,
    /// so this cannot fail and cannot reinterpret it: the region is passed
    /// through as discrete capture pixels under a rejecting policy.
    ///
    /// # Errors
    ///
    /// Returns a geometry fault only when the view's own region is not
    /// representable as coordinate values, which a constructed view rules out.
    pub fn from_view(
        view: &'a FrameView,
        template: &'a PreparedTemplate,
        options: MatchOptions,
    ) -> Result<Self, mado_pilot_core::Error> {
        Ok(Self {
            frame: view.frame(),
            selection: RegionSelection::pixels(view.region(), ClipPolicy::Reject)?,
            template,
            options,
        })
    }

    /// Returns the exact source frame.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        self.frame
    }

    /// Returns the region selection.
    #[must_use]
    pub const fn selection(&self) -> RegionSelection {
        self.selection
    }

    /// Returns the prepared template.
    #[must_use]
    pub const fn template(&self) -> &PreparedTemplate {
        self.template
    }

    /// Returns the options.
    #[must_use]
    pub const fn options(&self) -> MatchOptions {
        self.options
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::{ClipPolicy, CoordinateSpace, PixelRect};

    use super::{MatchOptions, RegionSelection, Suppression};
    use crate::fault::VisionFault;
    use crate::template::MatchDefaults;

    fn options() -> MatchOptions {
        MatchOptions::from_defaults(MatchDefaults::new(0.8, 5).expect("valid"))
    }

    #[test]
    fn options_start_from_the_templates_authored_defaults() {
        let options = options();

        assert_eq!(options.min_score(), 0.8);
        assert_eq!(options.max_results(), 5);
        assert_eq!(options.suppression(), Suppression::DropOverlapping);
    }

    #[test]
    fn an_out_of_range_threshold_is_refused_before_it_reaches_a_backend() {
        assert_eq!(
            options().with_min_score(1.1),
            Err(VisionFault::InvalidMatchScore)
        );
        assert_eq!(
            options().with_min_score(f64::NAN),
            Err(VisionFault::InvalidMatchScore)
        );
        assert_eq!(
            options().with_min_score(f64::INFINITY),
            Err(VisionFault::InvalidMatchScore)
        );
        assert!(options().with_min_score(0.0).is_ok());
        assert!(options().with_min_score(1.0).is_ok());
    }

    #[test]
    fn a_zero_result_limit_is_refused() {
        assert_eq!(
            options().with_max_results(0),
            Err(VisionFault::InvalidMatchResultLimit)
        );
    }

    #[test]
    fn a_pixel_region_becomes_a_capture_pixel_selection() {
        let region = PixelRect::new(10, 20, 30, 40).expect("valid");
        let selection = RegionSelection::pixels(region, ClipPolicy::Reject).expect("valid");

        let RegionSelection::Region { rect, policy } = selection else {
            panic!("a pixel region selects a region");
        };
        assert_eq!(rect.space(), CoordinateSpace::CapturePixels);
        assert_eq!(policy, ClipPolicy::Reject);
        assert_eq!((rect.left(), rect.top()), (10.0, 20.0));
    }
}
