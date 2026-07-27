//! What a completed search reports, and why it keeps working afterwards.
//!
//! A result owns everything it says. It copies the source frame's complete
//! identity and its transform snapshot rather than borrowing the frame, so
//! releasing the frame, the session, the prepared template, or the asset
//! package the template came from cannot change or invalidate it. That is also
//! what makes a result safe to hand across the C ABI later, where a borrowed
//! frame would become a lifetime rule stated only in prose.
//!
//! Bounds are always half-open rectangles in the complete source frame's
//! capture pixels, never relative to the searched region. A caller that asked
//! about a corner of a frame gets answers it can compare with answers about the
//! whole frame.

use std::cmp::Ordering;

use mado_pilot_core::{FrameStamp, PixelRect, TransformSnapshot};

use crate::backend::BackendDescriptor;
use crate::template::TemplateId;

/// One reported match.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    template: TemplateId,
    bounds: PixelRect,
    score: f64,
}

impl Match {
    pub(crate) const fn new(template: TemplateId, bounds: PixelRect, score: f64) -> Self {
        Self {
            template,
            bounds,
            score,
        }
    }

    /// Returns the identity of the template that matched.
    #[must_use]
    pub const fn template(&self) -> &TemplateId {
        &self.template
    }

    /// Returns the match's half-open bounds in full-frame capture pixels.
    #[must_use]
    pub const fn bounds(&self) -> PixelRect {
        self.bounds
    }

    /// Returns the public score, which is finite and inside `0.0..=1.0`.
    #[must_use]
    pub const fn score(&self) -> f64 {
        self.score
    }

    /// Compares two matches in the canonical public order.
    ///
    /// Descending score first, then ascending top, left, bottom, and right
    /// edges, then template identity. Every tie is broken by a value both
    /// release targets compute identically, which is what lets two hosts agree
    /// on ordering without agreeing on the last bit of a float.
    ///
    /// Scores are compared with a total order rather than a partial one. They
    /// are validated finite before they reach here, so the two agree — and a
    /// total order cannot panic if that validation is ever weakened.
    pub(crate) fn canonical_order(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.bounds.top().cmp(&other.bounds.top()))
            .then_with(|| self.bounds.left().cmp(&other.bounds.left()))
            .then_with(|| self.bounds.bottom().cmp(&other.bounds.bottom()))
            .then_with(|| self.bounds.right().cmp(&other.bounds.right()))
            .then_with(|| self.template.cmp(&other.template))
    }
}

/// The immutable outcome of one search.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    stamp: FrameStamp,
    transform: TransformSnapshot,
    searched: PixelRect,
    backend: BackendDescriptor,
    matches: Vec<Match>,
}

impl MatchResult {
    pub(crate) const fn new(
        stamp: FrameStamp,
        transform: TransformSnapshot,
        searched: PixelRect,
        backend: BackendDescriptor,
        matches: Vec<Match>,
    ) -> Self {
        Self {
            stamp,
            transform,
            searched,
            backend,
            matches,
        }
    }

    /// Returns the complete identity of the exact frame that was searched.
    #[must_use]
    pub const fn stamp(&self) -> FrameStamp {
        self.stamp
    }

    /// Returns the transform snapshot that frame carried.
    ///
    /// It is the snapshot taken at the searched frame, not the session's
    /// current one, so converting a match's bounds later answers the question
    /// the frame was captured under.
    #[must_use]
    pub const fn transform(&self) -> &TransformSnapshot {
        &self.transform
    }

    /// Returns the effective search region, in full-frame capture pixels.
    ///
    /// After any clipping. A caller comparing two results can tell whether they
    /// asked the same question.
    #[must_use]
    pub const fn searched(&self) -> PixelRect {
        self.searched
    }

    /// Returns the backend that produced the scores.
    #[must_use]
    pub const fn backend(&self) -> &BackendDescriptor {
        &self.backend
    }

    /// Returns the matches, in canonical order.
    ///
    /// An empty slice is a successful search that found nothing, which is a
    /// different answer from a failure and is never reported as one.
    #[must_use]
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }

    /// Returns the highest-scoring match, if any.
    #[must_use]
    pub fn best(&self) -> Option<&Match> {
        self.matches.first()
    }

    /// Reports whether the search found nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use mado_pilot_core::PixelRect;

    use super::Match;
    use crate::template::TemplateId;

    fn candidate(score: f64, left: i32, top: i32, name: &str) -> Match {
        Match::new(
            TemplateId::new(name).expect("non-empty"),
            PixelRect::new(left, top, left + 10, top + 10).expect("valid"),
            score,
        )
    }

    fn ordered(mut matches: Vec<Match>) -> Vec<String> {
        matches.sort_by(Match::canonical_order);
        matches
            .iter()
            .map(|found| {
                format!(
                    "{}@{},{}={}",
                    found.template(),
                    found.bounds().left(),
                    found.bounds().top(),
                    found.score()
                )
            })
            .collect()
    }

    #[test]
    fn a_higher_score_comes_first() {
        assert_eq!(
            ordered(vec![
                candidate(0.5, 0, 0, "a"),
                candidate(0.9, 40, 40, "a"),
                candidate(0.7, 20, 20, "a"),
            ]),
            vec!["a@40,40=0.9", "a@20,20=0.7", "a@0,0=0.5"]
        );
    }

    #[test]
    fn equal_scores_order_by_top_then_left() {
        assert_eq!(
            ordered(vec![
                candidate(0.8, 30, 10, "a"),
                candidate(0.8, 10, 10, "a"),
                candidate(0.8, 20, 5, "a"),
            ]),
            vec!["a@20,5=0.8", "a@10,10=0.8", "a@30,10=0.8"]
        );
    }

    #[test]
    fn identical_geometry_orders_by_template_identity() {
        assert_eq!(
            ordered(vec![
                candidate(0.8, 0, 0, "zebra"),
                candidate(0.8, 0, 0, "apple")
            ]),
            vec!["apple@0,0=0.8", "zebra@0,0=0.8"]
        );
    }

    #[test]
    fn the_order_is_total_so_repeated_sorts_do_not_move_anything() {
        let first = vec![
            candidate(0.8, 30, 10, "b"),
            candidate(0.8, 30, 10, "a"),
            candidate(0.9, 0, 0, "c"),
        ];
        let second = vec![
            candidate(0.9, 0, 0, "c"),
            candidate(0.8, 30, 10, "a"),
            candidate(0.8, 30, 10, "b"),
        ];

        assert_eq!(ordered(first), ordered(second));
    }
}
