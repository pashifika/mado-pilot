//! Canonical peaks from a request-owned correlation map.
//!
//! Order: descending public score, then row-major ties. One result scans once;
//! larger budgets use one winner/upper bound per row with lazy repair.
//! Suppression mutates the score map without a dense copy. Scanning and
//! suppression check caller authority in bounded chunks, including final commit.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use mado_pilot_core::operation::Operation;
use mado_pilot_core::{Interruption, OperationContext};
use mado_pilot_vision::Suppression;

/// Maximum scanned or suppressed offsets between operation checkpoints.
const CHECKPOINT_OFFSETS: usize = 4_096;

/// One extracted peak: an offset from the searched region's origin, and the
/// public score at that offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Peak {
    /// Columns from the searched region's left edge.
    pub(crate) left: usize,
    /// Rows from the searched region's top edge.
    pub(crate) top: usize,
    /// The score, already mapped onto the public range.
    pub(crate) score: f64,
}

/// Maps one raw `TM_CCOEFF_NORMED` value onto the public score range.
///
/// The two ends are clamped for different reasons. Below zero the clamp is a
/// decision: `TM_CCOEFF_NORMED` reaches minus one for a perfectly *inverted*
/// pattern, and the public score answers "how much like the template is this",
/// where an inverted pattern and an unrelated one are both "not at all". Above
/// one the clamp is only a rounding guard, because normalized correlation cannot
/// exceed one except by floating-point error.
///
/// Rescaling the full range onto `0.0..=1.0` would make a public score of `0.5`
/// mean "no correlation", weakening the meaning of callers' thresholds.
///
/// A non-finite raw value is not this function's concern; [`peaks`] skips those
/// offsets before mapping them.
pub(crate) fn public_score(raw: f32) -> f64 {
    f64::from(raw).clamp(0.0, 1.0)
}

/// What one peak search covers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeakSearch {
    /// Offsets per row in the score map.
    pub(crate) width: usize,
    /// Rows in the score map.
    pub(crate) height: usize,
    /// The template's width, which is the horizontal overlap window.
    pub(crate) template_width: usize,
    /// The template's height, which is the vertical overlap window.
    pub(crate) template_height: usize,
    /// The lowest public score worth reporting.
    pub(crate) min_score: f64,
    /// The maximum candidates emitted to the matcher. This equals the public
    /// result limit and is safe because extraction follows the same canonical
    /// order and requested suppression before consuming one budget slot.
    pub(crate) candidate_budget: usize,
    /// The request's overlap policy. Only `DropOverlapping` may remove neighbors.
    pub(crate) suppression: Suppression,
}

/// Selects the canonical prefix from mutable, row-major score scratch.
///
/// Non-finite offsets are skipped. Suppression marks removed offsets as `NAN`
/// only when another selection is required; interruption discards a partial prefix.
///
/// # Errors
///
/// Returns cancellation or deadline interruption at admission, bounded work
/// checkpoints, or final commit.
pub(crate) fn peaks(
    scores: &mut [f32],
    search: PeakSearch,
    operation: &OperationContext,
) -> Result<Vec<Peak>, Interruption> {
    let mut work = SelectionWork::new(operation)?;
    let Some(offsets) = search.width.checked_mul(search.height) else {
        return work.operation.commit(Vec::new());
    };
    if offsets == 0
        || scores.len() < offsets
        || search.candidate_budget == 0
        || search.template_width == 0
        || search.template_height == 0
    {
        return work.operation.commit(Vec::new());
    }

    let remaining = &mut scores[..offsets];
    let mut found = Vec::new();
    if search.candidate_budget == 1 {
        if let Some(winner) = work.greatest(remaining, 0, search.min_score)? {
            found = vec![winner.peak(search.width)];
        }
    } else {
        let mut rows = BinaryHeap::new();
        for (row, values) in remaining.chunks_exact(search.width).enumerate() {
            if let Some(winner) = work.greatest(values, row * search.width, search.min_score)? {
                rows.push(winner);
            }
        }

        if !rows.is_empty() && matches!(search.suppression, Suppression::KeepAll) {
            // Each live row guarantees a result; preserve geometric capacity growth.
            found.reserve(rows.len().min(search.candidate_budget).next_power_of_two());
        }

        while found.len() < search.candidate_budget {
            let Some(mut winner) = rows.peek_mut() else {
                break;
            };
            if !work.is_live(remaining[winner.index])? {
                let start = winner.index / search.width * search.width;
                if let Some(repaired) = work.greatest(
                    &remaining[start..start + search.width],
                    start,
                    search.min_score,
                )? {
                    *winner = repaired;
                } else {
                    std::collections::binary_heap::PeekMut::pop(winner);
                }
                continue;
            }

            // Removals can only lower a row's winner. A live heap maximum is
            // therefore the exact global winner, even above stale upper bounds.
            let peak = winner.peak(search.width);
            found.push(peak);
            if found.len() == search.candidate_budget {
                break;
            }
            work.suppress(remaining, search, peak)?;
        }
    }

    work.operation.commit(found)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RowWinner {
    index: usize,
    score: f64,
}

impl RowWinner {
    fn peak(self, width: usize) -> Peak {
        Peak {
            left: self.index % width,
            top: self.index / width,
            score: self.score,
        }
    }
}

// Winners contain only finite public scores; signed zeros are equal.
impl Eq for RowWinner {}

impl Ord for RowWinner {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.score > other.score {
            Ordering::Greater
        } else if self.score < other.score {
            Ordering::Less
        } else {
            other.index.cmp(&self.index)
        }
    }
}

impl PartialOrd for RowWinner {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct SelectionWork<'context> {
    operation: Operation<'context>,
    since_checkpoint: usize,
    #[cfg(feature = "benchmark-instrumentation")]
    observation: crate::benchmark_instrumentation::ScoreObservation,
}

impl<'context> SelectionWork<'context> {
    fn new(context: &'context OperationContext) -> Result<Self, Interruption> {
        Ok(Self {
            operation: Operation::admit(context)?,
            since_checkpoint: 0,
            #[cfg(feature = "benchmark-instrumentation")]
            observation: crate::benchmark_instrumentation::ScoreObservation::new(0),
        })
    }

    fn advance(&mut self, offsets: usize) -> Result<(), Interruption> {
        self.since_checkpoint += offsets;
        if self.since_checkpoint == CHECKPOINT_OFFSETS {
            self.since_checkpoint = 0;
            self.operation.checkpoint()?;
        }
        Ok(())
    }

    fn is_live(&mut self, value: f32) -> Result<bool, Interruption> {
        let live = value.is_finite();
        #[cfg(feature = "benchmark-instrumentation")]
        self.observation.scan(1);
        self.advance(1)?;
        Ok(live)
    }

    fn greatest(
        &mut self,
        values: &[f32],
        first_index: usize,
        min_score: f64,
    ) -> Result<Option<RowWinner>, Interruption> {
        let mut best: Option<RowWinner> = None;
        let mut best_score = f64::NEG_INFINITY;
        let mut scanned = 0;
        while scanned < values.len() {
            let count = (CHECKPOINT_OFFSETS - self.since_checkpoint).min(values.len() - scanned);
            for (column, &value) in values[scanned..scanned + count].iter().enumerate() {
                // Clamping cannot lift a raw value above an existing public
                // winner. Reject losers before normalization, but compare the
                // public score again to preserve ties above one.
                if !value.is_finite() || f64::from(value) <= best_score {
                    continue;
                }
                let score = public_score(value);
                if score > best_score {
                    best_score = score;
                    best = Some(RowWinner {
                        index: first_index + scanned + column,
                        score,
                    });
                    // The first public maximum cannot be outranked by a later offset.
                    if score >= 1.0 {
                        let visited = column + 1;
                        #[cfg(feature = "benchmark-instrumentation")]
                        self.observation.scan(visited);
                        self.advance(visited)?;
                        return Ok(best.filter(|winner| winner.score >= min_score));
                    }
                }
            }
            #[cfg(feature = "benchmark-instrumentation")]
            self.observation.scan(count);
            scanned += count;
            self.advance(count)?;
        }
        Ok(best.filter(|winner| winner.score >= min_score))
    }

    fn suppress(
        &mut self,
        remaining: &mut [f32],
        search: PeakSearch,
        peak: Peak,
    ) -> Result<(), Interruption> {
        if search.suppression != Suppression::DropOverlapping {
            remaining[peak.top * search.width + peak.left] = f32::NAN;
            return self.advance(1);
        }

        // Placements overlap iff both offset differences are strictly smaller
        // than the template extent. Edge-touching placements remain available.
        let first_row = peak.top.saturating_sub(search.template_height - 1);
        let last_row = peak
            .top
            .saturating_add(search.template_height - 1)
            .min(search.height - 1);
        let first_column = peak.left.saturating_sub(search.template_width - 1);
        let last_column = peak
            .left
            .saturating_add(search.template_width - 1)
            .min(search.width - 1);

        for row in first_row..=last_row {
            let mut start = row * search.width + first_column;
            let end = row * search.width + last_column + 1;
            while start < end {
                let count = (CHECKPOINT_OFFSETS - self.since_checkpoint).min(end - start);
                remaining[start..start + count].fill(f32::NAN);
                start += count;
                self.advance(count)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use mado_pilot_core::{
        CancellationToken, Clock, Interruption, MonotonicInstant, OperationContext,
    };
    use mado_pilot_vision::Suppression;

    use super::{CHECKPOINT_OFFSETS, Peak, PeakSearch, peaks, public_score};

    /// A deadline no sweep below can reach.
    const UNREACHABLE_MILLIS: u64 = 3_600_000;

    /// A clock that advances one millisecond every time it is read.
    ///
    /// Deadlines select successive operation checkpoints without wall-clock
    /// races. Tests sweep the observed checkpoints, not a fixed read count.
    #[derive(Debug)]
    struct TickingClock {
        reads: AtomicU64,
    }

    impl TickingClock {
        const fn new() -> Self {
            Self {
                reads: AtomicU64::new(0),
            }
        }

        fn reads(&self) -> u64 {
            self.reads.load(Ordering::Relaxed)
        }
    }

    impl Clock for TickingClock {
        fn now(&self) -> MonotonicInstant {
            let tick = self.reads.fetch_add(1, Ordering::Relaxed);
            MonotonicInstant::from_origin(Duration::from_millis(tick))
        }
    }

    /// A ticking clock that cancels `token` once it has been read `after` times.
    #[derive(Debug)]
    struct CancellingClock {
        reads: AtomicU64,
        after: u64,
        token: CancellationToken,
    }

    impl CancellingClock {
        const fn new(token: CancellationToken, after: u64) -> Self {
            Self {
                reads: AtomicU64::new(0),
                after,
                token,
            }
        }
    }

    impl Clock for CancellingClock {
        fn now(&self) -> MonotonicInstant {
            let tick = self.reads.fetch_add(1, Ordering::Relaxed);
            if tick >= self.after {
                self.token.cancel();
            }
            MonotonicInstant::from_origin(Duration::from_millis(tick))
        }
    }

    /// Extracts under a context that never interrupts.
    fn extract(scores: &[f32], search: PeakSearch) -> Vec<Peak> {
        peaks(&mut scores.to_vec(), search, &OperationContext::new())
            .expect("nothing interrupts extraction")
    }

    /// Wide rows make repeated lazy repairs cross operation checkpoints.
    fn permissive_search() -> (Vec<f32>, PeakSearch) {
        let width = CHECKPOINT_OFFSETS / 2 + 3;
        let height = 3;
        let scores = (0..width * height)
            .map(|offset| {
                let column = u16::try_from(offset % width).expect("the row fits in u16");
                1.0 - f32::from(column) / f32::from(u16::MAX)
            })
            .collect();

        (
            scores,
            PeakSearch {
                width,
                height,
                template_width: 1,
                template_height: 1,
                min_score: 0.0,
                candidate_budget: 12,
                suppression: Suppression::KeepAll,
            },
        )
    }

    /// Counts the context reads of an extraction that nothing interrupts.
    fn reads_for(scores: &[f32], search: PeakSearch) -> u64 {
        let clock = Arc::new(TickingClock::new());
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                UNREACHABLE_MILLIS,
            )));

        peaks(&mut scores.to_vec(), search, &context)
            .expect("the search finishes when nothing interrupts it");

        clock.reads()
    }

    /// Observes interrupted scratch writes, not just rejection at final commit.
    fn assert_bounded_partial_suppression(
        scores: &[f32],
        search: PeakSearch,
        max_suppressed_between_checkpoints: usize,
    ) {
        assert!(scores.iter().all(|value| value.is_finite()));
        let mut complete = scores.to_vec();
        peaks(&mut complete, search, &OperationContext::new())
            .expect("the uninterrupted search finishes");
        let total_suppressed = complete.iter().filter(|value| value.is_nan()).count();
        assert!(
            total_suppressed > max_suppressed_between_checkpoints,
            "the fixture must distinguish loop checkpoints from final commit"
        );
        let reads = reads_for(scores, search);

        for interruption in [Interruption::DeadlineExceeded, Interruption::Cancelled] {
            let points = if interruption == Interruption::Cancelled {
                reads.saturating_sub(1)
            } else {
                reads
            };
            let mut observed_suppression = vec![0, total_suppressed];
            for point in 0..points {
                let context = if interruption == Interruption::DeadlineExceeded {
                    OperationContext::new()
                        .with_clock(Arc::new(TickingClock::new()))
                        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(point)))
                } else {
                    let token = CancellationToken::new();
                    OperationContext::new()
                        .with_clock(Arc::new(CancellingClock::new(token.clone(), point)))
                        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                            UNREACHABLE_MILLIS,
                        )))
                        .with_cancellation(token)
                };
                let mut remaining = scores.to_vec();
                assert_eq!(
                    peaks(&mut remaining, search, &context).err(),
                    Some(interruption),
                    "{interruption:?} on context read {point} must stop {search:?}"
                );
                observed_suppression.push(remaining.iter().filter(|value| value.is_nan()).count());
            }
            observed_suppression.sort_unstable();
            observed_suppression.dedup();
            assert!(
                observed_suppression.len() > 2,
                "{interruption:?} must be observable during partial suppression"
            );
            assert!(
                observed_suppression
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] <= max_suppressed_between_checkpoints),
                "{interruption:?} must bound additional suppression: {observed_suppression:?}"
            );
        }
    }

    /// A search over a `width` by `height` map for a 2 by 2 template.
    fn search(width: usize, height: usize) -> PeakSearch {
        PeakSearch {
            width,
            height,
            template_width: 2,
            template_height: 2,
            min_score: 0.5,
            candidate_budget: 8,
            suppression: Suppression::DropOverlapping,
        }
    }

    /// The original repeated full-map greedy selection, independent of the row
    /// index and production suppression arithmetic.
    fn reference(scores: &[f32], search: PeakSearch) -> Vec<Peak> {
        let Some(offsets) = search.width.checked_mul(search.height) else {
            return Vec::new();
        };
        if offsets == 0
            || scores.len() < offsets
            || search.candidate_budget == 0
            || search.template_width == 0
            || search.template_height == 0
        {
            return Vec::new();
        }
        let mut remaining = scores[..offsets].to_vec();
        let mut found = Vec::new();
        while found.len() < search.candidate_budget {
            let mut best: Option<(usize, f64)> = None;
            for (index, &raw) in remaining.iter().enumerate() {
                if raw.is_finite() {
                    let score = f64::from(raw).clamp(0.0, 1.0);
                    if best.is_none_or(|(_, current)| score > current) {
                        best = Some((index, score));
                    }
                }
            }
            let Some((index, score)) = best else {
                break;
            };
            if score < search.min_score {
                break;
            }
            let left = index % search.width;
            let top = index / search.width;
            found.push(Peak { left, top, score });
            for (other, value) in remaining.iter_mut().enumerate() {
                let remove = if search.suppression == Suppression::DropOverlapping {
                    (other % search.width).abs_diff(left) < search.template_width
                        && (other / search.width).abs_diff(top) < search.template_height
                } else {
                    other == index
                };
                if remove {
                    *value = f32::NAN;
                }
            }
        }
        found
    }

    fn assert_canonical(scores: &[f32], search: PeakSearch) {
        assert_eq!(
            extract(scores, search),
            reference(scores, search),
            "canonical prefix for {search:?}"
        );
    }

    fn interruption_cases() -> Vec<(Vec<f32>, PeakSearch)> {
        let (scores, multi) = permissive_search();
        let overlapping = PeakSearch {
            template_width: 3,
            template_height: 2,
            suppression: Suppression::DropOverlapping,
            ..multi
        };
        let extent = CHECKPOINT_OFFSETS * 3 + 7;
        let wide = PeakSearch {
            width: extent,
            height: 1,
            template_width: usize::MAX,
            template_height: usize::MAX,
            candidate_budget: 2,
            suppression: Suppression::DropOverlapping,
            ..multi
        };
        let tall = PeakSearch {
            width: 1,
            height: extent,
            candidate_budget: 16,
            ..multi
        };
        let one = PeakSearch {
            candidate_budget: 1,
            ..wide
        };
        let mut boundary_maximum = vec![0.75; extent];
        boundary_maximum[CHECKPOINT_OFFSETS - 1] = 1.0;
        vec![
            (scores.clone(), multi),
            (scores, overlapping),
            (vec![0.75; extent], wide),
            (vec![0.75; extent], tall),
            (vec![1.0; extent], one),
            (boundary_maximum, one),
            (vec![1.0; extent], wide),
            (vec![1.0; extent], tall),
            (vec![0.75; 2], search(2, 1)),
            (vec![f32::NAN; extent], wide),
            (Vec::new(), search(0, 0)),
        ]
    }

    #[test]
    fn the_negative_half_of_the_correlation_range_clamps_to_no_match() {
        assert_eq!(public_score(-1.0), 0.0);
        assert_eq!(public_score(-0.25), 0.0);
        assert_eq!(public_score(0.0), 0.0);
    }

    #[test]
    fn a_score_inside_the_public_range_is_carried_through_unchanged() {
        assert_eq!(public_score(0.5), 0.5);
        assert_eq!(public_score(1.0), 1.0);
    }

    #[test]
    fn rounding_past_one_is_clamped_rather_than_published() {
        assert_eq!(public_score(1.000_000_1), 1.0);
    }

    #[test]
    fn the_greatest_offset_is_reported_first() {
        let scores = [0.6, 0.9, 0.7, 0.55];
        let mut search = search(4, 1);
        search.template_width = 1;
        search.template_height = 1;

        let found = extract(&scores, search);

        assert_eq!(
            found[0],
            Peak {
                left: 1,
                top: 0,
                score: 0.899_999_976_158_142_1
            }
        );
        assert_eq!(
            found.len(),
            4,
            "a one-pixel template suppresses only itself"
        );
    }

    #[test]
    fn offsets_that_overlap_the_peak_are_not_reported_again() {
        // A 2 by 2 template at (1, 1) overlaps every offset in (0..=2, 0..=2), so
        // the hill around 0.99 is reported once. The last row is below the
        // threshold so that only suppression decides the outcome.
        let scores = [
            0.90, 0.91, 0.92, 0.60, //
            0.93, 0.99, 0.94, 0.61, //
            0.95, 0.96, 0.97, 0.62, //
            0.10, 0.10, 0.10, 0.98,
        ];

        let found = extract(&scores, search(4, 4));

        assert_eq!(
            found
                .iter()
                .map(|peak| (peak.left, peak.top))
                .collect::<Vec<_>>(),
            vec![(1, 1), (3, 3), (3, 1)],
            "each survivor is clear of every earlier survivor's extent"
        );
    }

    #[test]
    fn keep_all_does_not_suppress_overlapping_offsets() {
        let scores = [0.90, 0.80, 0.70];
        let mut search = search(3, 1);
        search.template_height = 1;
        search.suppression = Suppression::KeepAll;

        let found = extract(&scores, search);

        assert_eq!(
            found.iter().map(|peak| peak.left).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "overlapping offsets remain candidates when the request keeps all"
        );
    }

    #[test]
    fn equal_scores_are_taken_in_row_then_column_order() {
        let scores = [
            0.50, 0.50, 0.80, //
            0.80, 0.50, 0.50, //
            0.50, 0.50, 0.50,
        ];
        let mut search = search(3, 3);
        search.min_score = 0.8;

        let found = extract(&scores, search);

        assert_eq!(
            found
                .iter()
                .map(|peak| (peak.left, peak.top))
                .collect::<Vec<_>>(),
            vec![(2, 0), (0, 1)],
            "the earlier row wins the tie, and neither suppresses the other"
        );
    }

    #[test]
    fn offsets_below_the_threshold_are_not_reported() {
        let scores = [0.10, 0.20, 0.30, 0.40];
        let mut search = search(4, 1);
        search.template_width = 1;
        search.template_height = 1;

        assert!(extract(&scores, search).is_empty());
    }

    #[test]
    fn the_result_limit_stops_the_search() {
        let scores = [0.90, 0.91, 0.92, 0.93];
        let mut search = search(4, 1);
        search.template_width = 1;
        search.template_height = 1;
        search.candidate_budget = 2;

        let found = extract(&scores, search);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].left, 3);
        assert_eq!(found[1].left, 2);
    }

    #[test]
    fn a_public_score_tie_is_broken_before_the_backend_result_budget() {
        let scores = [1.0, 1.000_000_1];
        let mut search = search(2, 1);
        search.template_width = 1;
        search.template_height = 1;
        search.min_score = 1.0;
        search.candidate_budget = 1;
        search.suppression = Suppression::KeepAll;

        let found = extract(&scores, search);

        assert_eq!(
            found,
            vec![Peak {
                left: 0,
                top: 0,
                score: 1.0,
            }],
            "equal public scores use canonical geometry before the budget truncates"
        );
    }

    #[test]
    fn a_degenerate_offset_is_skipped_rather_than_reported() {
        let scores = [f32::NAN, 0.90, f32::INFINITY, 0.60];
        let mut search = search(4, 1);
        search.template_width = 1;
        search.template_height = 1;

        let found = extract(&scores, search);

        assert_eq!(
            found.iter().map(|peak| peak.left).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn a_negative_correlation_reaches_the_public_range_only_at_a_zero_threshold() {
        let scores = [-0.90, -0.10];
        let mut search = search(2, 1);
        search.template_width = 1;
        search.template_height = 1;
        search.min_score = 0.0;

        let found = extract(&scores, search);

        assert_eq!(
            found,
            vec![
                Peak {
                    left: 0,
                    top: 0,
                    score: 0.0
                },
                Peak {
                    left: 1,
                    top: 0,
                    score: 0.0
                },
            ],
            "clamped public-score ties use canonical geometry rather than raw correlation"
        );

        search.min_score = 0.000_001;
        assert!(extract(&scores, search).is_empty());
    }

    #[test]
    fn an_empty_map_reports_nothing() {
        assert!(extract(&[], search(0, 0)).is_empty());
        assert!(extract(&[0.9], search(0, 4)).is_empty());
    }

    #[test]
    fn a_map_shorter_than_its_declared_extent_reports_nothing() {
        assert!(extract(&[0.9, 0.9], search(4, 4)).is_empty());
    }

    #[test]
    fn row_winners_match_the_greedy_reference_across_score_and_geometry_boundaries() {
        let raw = [
            f32::NAN,
            f32::NEG_INFINITY,
            -2.0,
            -0.0,
            0.0,
            0.25,
            0.5,
            0.75,
            1.0,
            1.000_000_1,
            2.0,
            f32::INFINITY,
        ];
        for (width, height) in [(33, 1), (5, 31), (31, 5), (17, 19)] {
            for seed in 0..2 {
                let scores = (0..width * height)
                    .map(|index| raw[(index * 17 + index / width * 7 + seed) % raw.len()])
                    .collect::<Vec<_>>();
                for suppression in [Suppression::KeepAll, Suppression::DropOverlapping] {
                    for (template_width, template_height) in [(1, 1), (3, 2), (4, 5)] {
                        for min_score in [0.0, 0.5, 1.0] {
                            for candidate_budget in [1, 16, 256, usize::MAX] {
                                assert_canonical(
                                    &scores,
                                    PeakSearch {
                                        width,
                                        height,
                                        template_width,
                                        template_height,
                                        min_score,
                                        candidate_budget,
                                        suppression,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn finite_single_offsets_and_columns_have_nonempty_canonical_prefixes() {
        for scores in [
            &[0.5][..],
            &[0.75][..],
            &[1.000_000_1][..],
            &[0.5, 1.0, 1.25, 0.25, 0.75, 0.75, -0.0, 0.0, -2.0, 0.5][..],
        ] {
            for suppression in [Suppression::KeepAll, Suppression::DropOverlapping] {
                for template_height in [1, 2, 5, usize::MAX] {
                    for min_score in [0.0, 0.5, 1.0] {
                        for candidate_budget in [1, 2, 16, usize::MAX] {
                            let search = PeakSearch {
                                template_width: 1,
                                template_height,
                                min_score,
                                candidate_budget,
                                suppression,
                                ..search(1, scores.len())
                            };
                            let found = extract(scores, search);
                            if min_score <= 0.5 {
                                assert!(
                                    !found.is_empty(),
                                    "finite qualifying offsets must produce a prefix for {search:?}"
                                );
                            }
                            assert_eq!(found, reference(scores, search));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn explicit_result_limits_preserve_the_prefix_and_stop_at_survivors() {
        let scores = vec![0.75; 17 * 19];
        for suppression in [Suppression::KeepAll, Suppression::DropOverlapping] {
            for limit in [1, 16, 256, usize::MAX] {
                let search = PeakSearch {
                    template_width: 1,
                    template_height: 1,
                    candidate_budget: limit,
                    suppression,
                    ..search(17, 19)
                };
                let found = extract(&scores, search);
                assert_eq!(found.len(), limit.min(scores.len()));
                assert_eq!(found, reference(&scores, search));
            }
        }
    }

    #[test]
    fn signed_zeros_tie_by_row_and_column_not_sign() {
        for scores in [
            [-0.0, 0.0, -0.0, 0.0],
            [0.0, -0.0, 0.0, -0.0],
            [-0.0, 0.0, 0.0, -0.0],
        ] {
            let search = PeakSearch {
                candidate_budget: 3,
                min_score: 0.0,
                suppression: Suppression::KeepAll,
                ..search(2, 2)
            };
            let found = extract(&scores, search);
            assert_eq!(
                found
                    .iter()
                    .map(|peak| (peak.left, peak.top))
                    .collect::<Vec<_>>(),
                vec![(0, 0), (1, 0), (0, 1)]
            );
            assert_eq!(found, reference(&scores, search));
        }
    }

    #[test]
    fn threshold_equality_is_inclusive_and_all_nonfinite_maps_are_empty() {
        let search = PeakSearch {
            template_width: 1,
            template_height: 1,
            ..search(4, 1)
        };
        assert_eq!(
            extract(&[f32::NAN, 0.5, f32::NEG_INFINITY, 0.25], search),
            vec![Peak {
                left: 1,
                top: 0,
                score: 0.5,
            }]
        );
        assert!(
            extract(
                &[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::NAN],
                search
            )
            .is_empty()
        );
        assert!(extract(&[0.25; 4], search).is_empty());
    }

    #[test]
    fn thresholds_between_f32_scores_preserve_the_f64_canonical_prefix() {
        let scores = [-2.0, -0.0, 0.0, 0.5, 1.0, 1.25];
        for boundary in [0.0_f64, 0.5, 1.0] {
            for min_score in [boundary.next_down(), boundary, boundary.next_up()] {
                for candidate_budget in [1, 16] {
                    for suppression in [Suppression::KeepAll, Suppression::DropOverlapping] {
                        assert_canonical(
                            &scores,
                            PeakSearch {
                                min_score,
                                candidate_budget,
                                suppression,
                                ..search(scores.len(), 1)
                            },
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn touching_edges_survive_while_overlapping_neighbors_are_removed() {
        let scores = [1.0; 25];
        let search = PeakSearch {
            candidate_budget: usize::MAX,
            ..search(5, 5)
        };
        let found = extract(&scores, search);
        assert_eq!(
            found
                .iter()
                .map(|peak| (peak.left, peak.top))
                .collect::<Vec<_>>(),
            vec![
                (0, 0),
                (2, 0),
                (4, 0),
                (0, 2),
                (2, 2),
                (4, 2),
                (0, 4),
                (2, 4),
                (4, 4),
            ]
        );
        assert_eq!(found, reference(&scores, search));
        assert_canonical(
            &scores,
            PeakSearch {
                template_width: usize::MAX,
                template_height: usize::MAX,
                ..search
            },
        );
    }

    #[test]
    fn suppression_repairs_stale_rows_before_publishing_the_next_winner() {
        let scores = [
            0.90, 0.89, 0.20, 0.80, 0.78, 0.10, //
            0.88, 0.87, 0.30, 0.79, 0.77, 0.20, //
            0.86, 0.85, 0.40, 0.76, 0.75, 0.20,
        ];
        let search = PeakSearch {
            min_score: 0.0,
            candidate_budget: 16,
            ..search(6, 3)
        };
        let found = extract(&scores, search);
        assert_eq!(
            found
                .iter()
                .take(4)
                .map(|peak| (peak.left, peak.top))
                .collect::<Vec<_>>(),
            vec![(0, 0), (0, 2), (3, 0), (3, 2)]
        );
        assert_eq!(found, reference(&scores, search));
    }

    #[test]
    fn invalid_extents_and_zero_limits_return_empty_without_large_allocations() {
        for search in [
            search(usize::MAX, 2),
            search(2, usize::MAX),
            search(4, 0),
            PeakSearch {
                template_width: 0,
                ..search(1, 1)
            },
            PeakSearch {
                template_height: 0,
                ..search(1, 1)
            },
            PeakSearch {
                candidate_budget: 0,
                ..search(1, 1)
            },
        ] {
            assert!(extract(&[1.0], search).is_empty());
        }
        let search = PeakSearch {
            candidate_budget: usize::MAX,
            ..search(1, 1)
        };
        assert_eq!(extract(&[1.0], search).len(), 1);
    }

    #[test]
    fn already_cancelled_and_expired_calls_retain_their_own_authority() {
        for (scores, search) in interruption_cases() {
            let token = CancellationToken::new();
            token.cancel();
            let cancelled = OperationContext::new().with_cancellation(token);
            assert_eq!(
                peaks(&mut scores.clone(), search, &cancelled).err(),
                Some(Interruption::Cancelled)
            );
            let expired = OperationContext::new()
                .with_clock(Arc::new(TickingClock::new()))
                .with_deadline(MonotonicInstant::from_origin(Duration::ZERO));
            assert_eq!(
                peaks(&mut scores.clone(), search, &expired).err(),
                Some(Interruption::DeadlineExceeded)
            );
            assert_canonical(&scores, search);
        }
    }

    #[test]
    fn no_deadline_anywhere_inside_extraction_reports_candidates() {
        for (scores, search) in interruption_cases() {
            for deadline in 0..reads_for(&scores, search) {
                let context = OperationContext::new()
                    .with_clock(Arc::new(TickingClock::new()))
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                        deadline,
                    )));
                assert_eq!(
                    peaks(&mut scores.clone(), search, &context).err(),
                    Some(Interruption::DeadlineExceeded),
                    "a deadline expiring on context read {deadline} must stop {search:?}"
                );
            }
        }
    }

    #[test]
    fn no_cancellation_anywhere_inside_extraction_reports_candidates() {
        for (scores, search) in interruption_cases() {
            let total = reads_for(&scores, search);
            // A clock callback's cancellation is visible at the next context
            // check; cancellation after the final check has no stronger fence.
            for point in 0..total.saturating_sub(1) {
                let token = CancellationToken::new();
                let clock = Arc::new(CancellingClock::new(token.clone(), point));
                let context = OperationContext::new()
                    .with_clock(clock)
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                        UNREACHABLE_MILLIS,
                    )))
                    .with_cancellation(token);
                assert_eq!(
                    peaks(&mut scores.clone(), search, &context).err(),
                    Some(Interruption::Cancelled),
                    "cancellation on context read {point} must stop {search:?}"
                );
            }
        }
    }

    #[test]
    fn a_large_overlap_window_can_stop_after_a_bounded_partial_suppression() {
        let scores = vec![0.75; CHECKPOINT_OFFSETS * 3 + 7];
        let search = PeakSearch {
            template_width: usize::MAX,
            candidate_budget: 2,
            ..search(scores.len(), 1)
        };
        assert_bounded_partial_suppression(&scores, search, CHECKPOINT_OFFSETS);
    }

    #[test]
    fn lazy_repairs_and_multiple_suppressions_remain_boundedly_interruptible() {
        let (scores, search) = permissive_search();
        // Repairs span over half a checkpoint. KeepAll removes one offset per
        // selection; each edge-row overlap window adds at most six new offsets.
        assert_bounded_partial_suppression(&scores, search, 2);
        assert_bounded_partial_suppression(
            &scores,
            PeakSearch {
                template_width: 3,
                template_height: 2,
                suppression: Suppression::DropOverlapping,
                ..search
            },
            12,
        );
    }
}
