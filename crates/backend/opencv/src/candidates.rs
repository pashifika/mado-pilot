//! Turning a dense correlation map into the candidates the matcher normalizes.
//!
//! `matchTemplate` scores every offset at which the template fits, so one real
//! match arrives as a smooth hill of thousands of high values rather than as one
//! finding. Materializing that entire dense map as public candidates would make
//! a permissive threshold consume unbounded result memory.
//!
//! The adapter therefore emits a bounded canonical prefix. It repeatedly takes
//! the greatest *public* score, breaking ties row-major exactly as the matcher
//! does. `DropOverlapping` removes the selected placement's overlap window;
//! `KeepAll` removes only the selected offset. The request's public result limit
//! is also the backend candidate budget, so extraction performs at most that many
//! map scans without changing any result the matcher could publish through that
//! limit. The matcher still reapplies thresholding, ordering, suppression, and
//! truncation as the public authority.
//!
//! That budget bounds the work but does not make it short: the public result
//! limit is a caller's `u32`, so extraction reads the operation context between
//! scans and stops on the caller's cancellation or deadline rather than running
//! to a budget the caller no longer wants paid.
//!
//! The arithmetic here is independent of OpenCV's native types, which makes the
//! bounded extraction rules directly testable and identical on both release
//! targets.

use mado_pilot_core::{Interruption, OperationContext};
use mado_pilot_vision::Suppression;

/// How many offsets extraction may compare between two reads of the operation
/// context.
///
/// Extraction has two costs and only one of them is open-ended. A single scan is
/// bounded by the score map, which is bounded by the searched region and so by
/// the frame; the *number* of scans is bounded only by the caller's result
/// limit, which the public API accepts up to `u32::MAX`. The checkpoint
/// therefore sits between scans, where the unbounded axis is.
///
/// Counting compared offsets rather than emitted candidates is what keeps the
/// check off a small search's critical path. Reading the context costs a clock
/// read; comparing this many offsets costs microseconds, two orders of magnitude
/// more, so the check cannot dominate. A map smaller than this is scanned
/// several times per check, and a map larger than it is checked once per scan.
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
/// Rescaling the full range onto `0.0..=1.0` instead was rejected: it would make
/// a public score of `0.5` mean "no correlation", so every caller's threshold
/// would quietly mean something weaker than it reads. See
/// `docs/adr/0003-opencv-matching-profile-and-public-score.md`.
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

/// Extracts the bounded candidate prefix from `scores`, greatest first.
///
/// `scores` is a row-major map of `search.width` by `search.height` values, one
/// per offset at which the template fits the searched region.
///
/// Selection reads the mapped public score. Two raw values that clamp to the
/// same public score therefore tie and are broken by row and then column, which
/// is the matcher's canonical geometry order for a fixed template extent.
///
/// An offset whose value is not finite is skipped rather than reported or
/// treated as a failure: `TM_CCOEFF_NORMED` normalizes by the variance of the
/// template and of the window, so a uniform window has no correlation to
/// express, and no correlation is not evidence of a match.
///
/// The work is bounded by `search.candidate_budget` scans of the map, and each
/// scan removes at least the selected offset.
///
/// # Errors
///
/// Returns the [`Interruption`] `operation` reports when cancellation or the
/// deadline lands between two scans. Extraction is pure arithmetic, so it stops
/// where it stands and reports nothing rather than finishing a prefix no caller
/// is waiting for. See [`CHECKPOINT_OFFSETS`] for how often the context is read.
pub(crate) fn peaks(
    scores: &[f32],
    search: PeakSearch,
    operation: &OperationContext,
) -> Result<Vec<Peak>, Interruption> {
    let offsets = search.width.saturating_mul(search.height);
    if offsets == 0
        || scores.len() < offsets
        || search.candidate_budget == 0
        || search.template_width == 0
        || search.template_height == 0
    {
        return Ok(Vec::new());
    }

    // Suppression writes into a copy so the caller's map stays intact, and it
    // writes `NAN` because the scan already skips non-finite offsets.
    let mut remaining = scores[..offsets].to_vec();
    let mut found = Vec::new();
    // Offsets compared since the context was last read. The caller checks the
    // context immediately before calling, so the first scan starts owing
    // nothing.
    let mut compared: usize = 0;

    while found.len() < search.candidate_budget {
        if compared >= CHECKPOINT_OFFSETS {
            if let Some(interruption) = operation.interruption() {
                return Err(interruption);
            }
            compared = 0;
        }
        compared = compared.saturating_add(offsets);

        let Some((left, top, score)) = greatest(&remaining, search.width) else {
            break;
        };
        if score < search.min_score {
            // Every remaining offset scores no higher than this one, so no
            // further scan can qualify.
            break;
        }
        found.push(Peak { left, top, score });
        if search.suppression == Suppression::DropOverlapping {
            suppress(&mut remaining, search, left, top);
        } else {
            // `KeepAll` — and any future policy the public matcher understands —
            // must reach that matcher without implicit overlap suppression.
            remaining[top * search.width + left] = f32::NAN;
        }
    }

    Ok(found)
}

/// Returns the greatest finite public score in `remaining`, scanning row-major
/// so the first of several public-score ties wins.
fn greatest(remaining: &[f32], width: usize) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;

    for (index, &value) in remaining.iter().enumerate() {
        if !value.is_finite() {
            continue;
        }
        let score = public_score(value);
        if best.is_none_or(|(_, _, current)| score > current) {
            best = Some((index % width, index / width, score));
        }
    }

    best
}

/// Removes every offset that would overlap a template placed at `(left, top)`.
///
/// Two placements of one template overlap exactly when their offsets differ by
/// less than the template's extent in both axes, so the suppressed window is
/// one template short of twice the template's size, centred on the peak.
fn suppress(remaining: &mut [f32], search: PeakSearch, left: usize, top: usize) {
    let first_row = top.saturating_sub(search.template_height - 1);
    let last_row = top
        .saturating_add(search.template_height - 1)
        .min(search.height - 1);
    let first_column = left.saturating_sub(search.template_width - 1);
    let last_column = left
        .saturating_add(search.template_width - 1)
        .min(search.width - 1);

    for row in first_row..=last_row {
        let start = row * search.width;
        for column in first_column..=last_column {
            remaining[start + column] = f32::NAN;
        }
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
    /// A deadline is then a count of context reads rather than a wall-clock
    /// wait, so "the deadline expired while the map was being rescanned" is an
    /// exact statement about which check observed it rather than a race against
    /// a busy machine.
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

        fn reads(&self) -> u64 {
            self.reads.load(Ordering::Relaxed)
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
        peaks(scores, search, &OperationContext::new()).expect("nothing interrupts extraction")
    }

    /// A search that keeps rescanning: one checkpoint interval of offsets, a one
    /// pixel template, no threshold, and no suppression, so every offset stays a
    /// candidate and the budget alone decides how many scans happen.
    fn permissive_search() -> (Vec<f32>, PeakSearch) {
        let width = 64;
        let height = CHECKPOINT_OFFSETS / width;
        let scores = (0..width * height)
            .map(|offset| {
                let index = u16::try_from(offset).expect("the map is smaller than u16::MAX");
                1.0 - f32::from(index) / f32::from(u16::MAX)
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

        peaks(scores, search, &context).expect("the search finishes when nothing interrupts it");

        clock.reads()
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
    fn the_context_is_read_while_the_map_is_being_scanned_rather_than_once() {
        let (scores, search) = permissive_search();

        assert!(
            reads_for(&scores, search) > 1,
            "a search whose budget outlives one checkpoint interval must read \
             the context more than once, or nothing inside the loop can stop it"
        );
    }

    #[test]
    fn no_deadline_anywhere_inside_extraction_reports_candidates() {
        let (scores, search) = permissive_search();
        let total = reads_for(&scores, search);

        for deadline in 0..total {
            let clock = Arc::new(TickingClock::new());
            let context = OperationContext::new().with_clock(clock).with_deadline(
                MonotonicInstant::from_origin(Duration::from_millis(deadline)),
            );

            assert_eq!(
                peaks(&scores, search, &context).err(),
                Some(Interruption::DeadlineExceeded),
                "a deadline expiring on context read {deadline} must stop extraction"
            );
        }
    }

    #[test]
    fn no_cancellation_anywhere_inside_extraction_reports_candidates() {
        let (scores, search) = permissive_search();
        let total = reads_for(&scores, search);

        // Cancelling on read `point` is observed by the next read, so the last
        // read that can still observe it is `total - 2`.
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
                peaks(&scores, search, &context).err(),
                Some(Interruption::Cancelled),
                "cancellation landing on context read {point} must stop extraction"
            );
        }
    }

    #[test]
    fn a_budget_the_map_cannot_fill_is_interrupted_rather_than_run_out() {
        // The public result limit is a caller's `u32`, and `KeepAll` with a zero
        // threshold qualifies every offset, so this is the shape that made
        // extraction run for hours: the budget, not the map, sets the length.
        let (scores, mut search) = permissive_search();
        search.candidate_budget = usize::try_from(u32::MAX).expect("a u32 fits a usize");
        let token = CancellationToken::new();
        let clock = Arc::new(CancellingClock::new(token.clone(), 0));
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                UNREACHABLE_MILLIS,
            )))
            .with_cancellation(token);

        assert_eq!(
            peaks(&scores, search, &context).err(),
            Some(Interruption::Cancelled)
        );
        assert_eq!(
            clock.reads(),
            1,
            "extraction stopped at the checkpoint after cancellation, not after \
             four billion scans"
        );
    }

    #[test]
    fn an_extraction_shorter_than_one_checkpoint_interval_never_reads_the_context() {
        let clock = Arc::new(TickingClock::new());
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
                UNREACHABLE_MILLIS,
            )));
        let mut search = search(2, 1);
        search.template_width = 1;
        search.template_height = 1;
        search.min_score = 0.0;
        search.suppression = Suppression::KeepAll;

        peaks(&[0.9, 0.8], search, &context).expect("nothing interrupts extraction");

        assert_eq!(
            clock.reads(),
            0,
            "checking per candidate would cost a clock read per handful of \
             comparisons on a map this small"
        );
    }
}
