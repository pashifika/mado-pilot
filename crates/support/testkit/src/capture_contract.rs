//! The capture contract suite every capture adapter must pass.
//!
//! Written as assertions over the public traits rather than over any
//! implementation, so an adapter passes it for the same reasons a caller can
//! rely on it. Each check panics with a message naming the rule it enforces,
//! because a contract failure is a defect in the adapter and not a value to be
//! handled.
//!
//! Every wait here carries a deadline, and every check is bounded a second time
//! from outside. This is the one suite that genuinely waits — a frame request
//! blocks until something is published — so an adapter that opens a session and
//! never publishes must fail the check that waited rather than hang the run
//! with no output naming it.
//!
//! The deadline is the bound an adapter applies to itself, and it is the whole
//! bound for one that consults it. An adapter that implements its own waiting
//! and never reads the deadline it was given applies none, and no ordering of
//! the checks avoids that, because every check calls into the adapter. So each
//! check also arms a watchdog, which ends the run naming the adapter's fault
//! rather than leaving it hung.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureProvider, DiscoveryRequest, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{
    CancellationToken, Clock, FrameOrder, IdentityIssuer, MonotonicInstant, OperationContext,
    Status,
};

/// How long any single check waits before it reports a hang as a failure.
///
/// Generous, because the bound exists to turn "never" into a diagnosis rather
/// than to time anything: no adapter that satisfies the contract comes near it,
/// and one that does not is not made to pass by a larger number.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// A deadline no sweep can reach: the clock would have to be read 3.6 million
/// times.
const UNREACHABLE_MILLIS: u64 = 3_600_000;

/// How long a check may run before the watchdog treats it as never returning.
///
/// [`CHECK_TIMEOUT`] and two seconds of grace: an adapter that consults the
/// deadline every context here carries returns by the first, so a call still
/// running past the second is a call that will not end on its own.
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(CHECK_TIMEOUT.as_secs() + 2);

/// The code the watchdog ends the process with — what a failing test exits with.
const WATCHDOG_EXIT: i32 = 101;

/// Bounds one check from outside the call it makes into the adapter.
///
/// A thread blocked inside an adapter cannot be interrupted from outside, so
/// the watchdog ends the process rather than the check. That is a blunt
/// instrument and it is the honest one: the alternative is a run that hangs and
/// names nothing, and the run is failing either way.
///
/// The check says it returned by dropping this, including while a contract
/// failure unwinds through it.
#[derive(Debug)]
struct Watchdog {
    /// Held for the length of the check; its disconnection is the signal.
    _returned: mpsc::Sender<()>,
}

impl Watchdog {
    /// Returns a watchdog that fails the run if `check` outlives its bound.
    fn guarding(check: &'static str) -> Self {
        let (returned, waiting) = mpsc::channel();
        std::thread::spawn(move || {
            if !matches!(
                waiting.recv_timeout(WATCHDOG_TIMEOUT),
                Err(RecvTimeoutError::Timeout)
            ) {
                return;
            }

            eprintln!(
                "capture contract: `{check}` has not returned in {WATCHDOG_TIMEOUT:?}.\n\
                 This adapter did not honour the deadline it was given: every operation \
                 context this suite passes expires after {CHECK_TIMEOUT:?}, so an adapter \
                 that consults its deadline cannot block for longer than that, whether or \
                 not it ever publishes a frame. This one is waiting for something nothing \
                 will tell it to stop waiting for.\n\
                 Ending the process with {WATCHDOG_EXIT}, because a call blocked inside an \
                 adapter cannot be interrupted and a run that hangs reports nothing."
            );
            std::process::exit(WATCHDOG_EXIT);
        });

        Self {
            _returned: returned,
        }
    }
}

/// Returns the context every check that can block uses.
fn bounded() -> OperationContext {
    OperationContext::new()
        .with_timeout(CHECK_TIMEOUT)
        .expect("a ten-second timeout is representable")
}

/// A clock that advances one millisecond per read and counts its reads.
///
/// A deadline against it is a count of context checks rather than a wall-clock
/// wait, which is what lets a check sweep every interruption point instead of
/// racing a timer. [`crate::ManualClock`] is the other shape — a clock a test
/// moves by hand — and neither substitutes for the other.
#[derive(Debug, Default)]
struct TickingClock {
    reads: AtomicU64,
}

impl TickingClock {
    /// Returns a clock at the domain origin.
    const fn new() -> Self {
        Self {
            reads: AtomicU64::new(0),
        }
    }

    /// Returns how many times the clock has been read.
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

/// Returns a context whose deadline expires on its `read`-th context check.
fn expiring_at(read: u64) -> OperationContext {
    OperationContext::new()
        .with_clock(Arc::new(TickingClock::new()))
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(read)))
}

/// Checks that a tracked capture fixture directory still matches its
/// `SHA256SUMS`, in both directions.
///
/// Re-exported here so a capture fixture's suite reaches it beside the contract
/// checks it already runs, rather than reaching past them into another module.
///
/// # Panics
///
/// As [`crate::fixture_checksums::verify`].
pub fn verify_fixture_checksums(root: &std::path::Path) {
    crate::fixture_checksums::verify(root);
}

/// Runs every contract check against `provider`.
///
/// `provider` must offer at least one target and must be able to publish at
/// least one frame without help. An adapter that needs a test to drive
/// publication runs the individual checks instead.
///
/// # Panics
///
/// Panics when the adapter violates the capture contract. A check that never
/// returns ends the process instead, for the reason the module documents.
pub fn run(provider: &dyn CaptureProvider) {
    discovery_is_provider_qualified(provider);
    discovery_filtering_narrows_without_reordering(provider);
    a_foreign_target_is_refused(provider);
    the_first_frame_is_the_start_of_the_stream(provider);
    repeated_latest_requests_return_one_identity(provider);
    derived_outputs_report_the_exact_source_frame(provider);
    an_already_cancelled_request_is_refused(provider);
    an_already_expired_request_is_refused(provider);
    no_deadline_inside_a_frame_request_produces_a_frame(provider);
    close_is_idempotent_and_retained_outputs_survive_it(provider);
}

/// Every discovered target is qualified by the provider that found it.
///
/// # Panics
///
/// Panics when a description names another provider.
pub fn discovery_is_provider_qualified(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("discovery_is_provider_qualified");
    let operation = bounded();
    let targets = provider.discover(&operation).expect("discovery succeeds");
    assert!(
        !targets.is_empty(),
        "a provider under contract test must offer a target"
    );
    for target in &targets {
        assert_eq!(
            target.provider(),
            provider.provider(),
            "a target description must be qualified by its own provider"
        );
        assert_eq!(target.id().provider(), provider.provider());
    }
}

/// A filter selects from what discovery listed, in the order it listed it.
///
/// The three properties a filter must have and cannot be allowed to lose: an
/// unfiltered request means what it always meant, a filter is a subset of the
/// unfiltered list rather than a second query, and it preserves order so a caller
/// that takes the first match takes the same target twice running.
///
/// # Panics
///
/// Panics when a filter reaches a target discovery did not list, when it reorders
/// the list, or when an unfiltered filtered request differs from discovery.
pub fn discovery_filtering_narrows_without_reordering(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("discovery_filtering_narrows_without_reordering");
    let operation = bounded();
    let listed = provider.discover(&operation).expect("discovery succeeds");
    assert!(
        !listed.is_empty(),
        "a provider under contract test must offer a target"
    );

    let unfiltered = provider
        .discover_matching(&DiscoveryRequest::new(), &operation)
        .expect("an unfiltered request succeeds");
    assert_eq!(
        unfiltered, listed,
        "a request with no filter means every target this provider listed"
    );

    // Filtering on the first target's own name can only ever select a subset that
    // includes it, whatever the provider is and however many targets it has.
    let wanted = listed[0].name().to_owned();
    let narrowed = provider
        .discover_matching(
            &DiscoveryRequest::new().with_name_containing(wanted.clone()),
            &operation,
        )
        .expect("a filtered request succeeds");

    assert!(
        narrowed.iter().all(|target| listed.contains(target)),
        "a filter selects from the provider's own result set and reaches nothing else"
    );
    assert!(
        narrowed.iter().any(|target| target.id() == listed[0].id()),
        "a filter on a target's own descriptive name must select that target"
    );
    assert!(
        narrowed
            .iter()
            .all(|target| target.name().contains(&wanted)),
        "a filtered list contains only targets that match the filter"
    );
    let expected_order: Vec<_> = listed
        .iter()
        .filter(|target| target.name().contains(&wanted))
        .cloned()
        .collect();
    assert_eq!(
        narrowed, expected_order,
        "filtering preserves the order discovery reported"
    );

    let repeated = provider
        .discover_matching(
            &DiscoveryRequest::new().with_name_containing(wanted),
            &operation,
        )
        .expect("a filtered request succeeds");
    assert_eq!(
        repeated, narrowed,
        "the same filter over an unchanged desktop selects the same targets"
    );
}

/// A target identity from another engine is refused rather than matched by name.
///
/// # Panics
///
/// Panics when the adapter opens a session for a foreign identity.
pub fn a_foreign_target_is_refused(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("a_foreign_target_is_refused");
    let operation = bounded();
    let other = IdentityIssuer::new();
    let foreign = other
        .issue_target(provider.provider())
        .expect("identity issued");

    let error = provider
        .open(foreign, &OpenRequest::new(), &operation)
        .expect_err("a foreign identity must not open a session");

    assert_eq!(
        error.status(),
        Status::InvalidArgument,
        "a foreign identity is an invalid argument, not a missing target"
    );
}

/// The first published frame starts the stream at epoch zero, sequence zero.
///
/// # Panics
///
/// Panics when the first frame is numbered differently or its descriptor and
/// pixels disagree.
pub fn the_first_frame_is_the_start_of_the_stream(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("the_first_frame_is_the_start_of_the_stream");
    let operation = bounded();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");

    assert_eq!(frame.stamp().epoch().value(), 0);
    assert_eq!(frame.stamp().sequence().value(), 0);
    assert_eq!(
        frame.stamp().stream(),
        session.description().stream(),
        "a frame must belong to the stream that published it"
    );
    assert_eq!(
        frame.transform().frame_extent(),
        frame.descriptor().extent(),
        "geometry and pixels must describe the same rectangle"
    );

    let mapping = frame
        .map(frame.descriptor().format(), &operation)
        .expect("a published frame maps");
    assert_eq!(mapping.bytes().len(), frame.descriptor().byte_len());
    session.close(&operation).expect("close succeeds");
}

/// Asking for the latest frame twice returns one identity, not two.
///
/// # Panics
///
/// Panics when the adapter renames a frame it has already published.
pub fn repeated_latest_requests_return_one_identity(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("repeated_latest_requests_return_one_identity");
    let operation = bounded();
    let session = open_first(provider);

    let first = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let second = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");

    assert_eq!(
        first.stamp(),
        second.stamp(),
        "a maintained frame must not be assigned a new identity per request"
    );
    assert_eq!(first.stamp().order(&second.stamp()), Ok(FrameOrder::Same));
    session.close(&operation).expect("close succeeds");
}

/// A view and a mapping report the frame they came from, not a newer one.
///
/// # Panics
///
/// Panics when a derived output loses or changes its source identity.
pub fn derived_outputs_report_the_exact_source_frame(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("derived_outputs_report_the_exact_source_frame");
    let operation = bounded();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let stamp = frame.stamp();

    let view = frame.full_view().expect("the whole frame is a valid view");
    assert_eq!(view.stamp(), stamp);

    let mapping = view
        .map(frame.descriptor().format(), &operation)
        .expect("a view maps");
    assert_eq!(mapping.stamp(), stamp);
    assert_eq!(mapping.transform().geometry(), stamp.geometry());

    session.close(&operation).expect("close succeeds");
}

/// A request whose token is already cancelled never produces a frame.
///
/// # Panics
///
/// Panics when the adapter admits work it should have refused.
pub fn an_already_cancelled_request_is_refused(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("an_already_cancelled_request_is_refused");
    let session = open_first(provider);
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = bounded().with_cancellation(token);

    let error = session
        .frame(&FrameRequest::latest(), &cancelled)
        .expect_err("an already cancelled request must not return a frame");

    assert_eq!(error.status(), Status::Cancelled);
    session.close(&bounded()).expect("close succeeds");
}

/// A request whose deadline has already passed never produces a frame.
///
/// The other half of the pair the cancellation check above starts. An adapter
/// that consults its cancellation token but never its deadline satisfies every
/// other check in this suite, and a wait that outlives its deadline is the
/// realistic defect in an adapter that implements its own waiting rather than
/// delegating it.
///
/// # Panics
///
/// Panics when the adapter admits work whose deadline has passed, or reports
/// the refusal as something other than an expired deadline.
pub fn an_already_expired_request_is_refused(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("an_already_expired_request_is_refused");
    let session = open_first(provider);

    let error = session
        .frame(&FrameRequest::latest(), &expiring_at(0))
        .expect_err("an already expired request must not return a frame");

    assert_eq!(error.status(), Status::DeadlineExceeded);
    session.close(&bounded()).expect("close succeeds");
}

/// No deadline anywhere inside a frame request produces a frame.
///
/// A sweep over every context read rather than one hand-picked point, which is
/// the shape `crates/automation/assets/tests/operation_context.rs` established
/// for the loader. The property worth pinning is not that a deadline expiring
/// at the second check is refused; it is that no deadline anywhere is answered
/// with a frame, and that the request reaches the context more than once, so
/// both admission and commit are covered rather than only the entry check.
///
/// # Panics
///
/// Panics when a request whose deadline expires part-way through still returns
/// a frame, when the refusal is reported as something else, or when the adapter
/// consults the context fewer than twice.
pub fn no_deadline_inside_a_frame_request_produces_a_frame(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("no_deadline_inside_a_frame_request_produces_a_frame");
    let session = open_first(provider);

    // One uninterrupted request, to learn how many times it reads the context.
    let clock = Arc::new(TickingClock::new());
    let unreachable = OperationContext::new()
        .with_clock(clock.clone())
        .with_deadline(MonotonicInstant::from_origin(Duration::from_millis(
            UNREACHABLE_MILLIS,
        )));
    session
        .frame(&FrameRequest::latest(), &unreachable)
        .expect("a frame is available when nothing interrupts the request");
    let reads = clock.reads();

    assert!(
        reads >= 2,
        "a frame request must consult its operation context before it is \
         admitted and again before its frame is committed, but this adapter \
         consulted it {reads} time(s)"
    );

    for read in 0..reads {
        let error = session
            .frame(&FrameRequest::latest(), &expiring_at(read))
            .expect_err("a deadline that expires during a request produces no frame");

        assert_eq!(
            error.status(),
            Status::DeadlineExceeded,
            "a deadline expiring at context read {read} must be reported as one"
        );
    }

    session.close(&bounded()).expect("close succeeds");
}

/// Close can be repeated, and what the caller already holds survives it.
///
/// # Panics
///
/// Panics when a second close fails, when a closed session still serves frames,
/// or when a retained frame or mapping is disturbed by close.
pub fn close_is_idempotent_and_retained_outputs_survive_it(provider: &dyn CaptureProvider) {
    let _watchdog = Watchdog::guarding("close_is_idempotent_and_retained_outputs_survive_it");
    let operation = bounded();
    let session = open_first(provider);
    let frame = session
        .frame(&FrameRequest::latest(), &operation)
        .expect("a frame is available");
    let format = frame.descriptor().format();
    let mapping = frame.map(format, &operation).expect("a frame maps");
    let expected: Vec<u8> = mapping.bytes().to_vec();
    let stamp = frame.stamp();

    session.close(&operation).expect("close succeeds");
    session
        .close(&operation)
        .expect("close is safe to repeat after it has succeeded");
    assert!(session.is_closed());

    let error = session
        .frame(&FrameRequest::latest(), &operation)
        .expect_err("a closed session serves no frames");
    assert_eq!(error.status(), Status::Closed);

    assert_eq!(
        frame.stamp(),
        stamp,
        "close must not disturb a retained frame"
    );
    assert_eq!(
        mapping.bytes(),
        expected.as_slice(),
        "close must not disturb a completed mapping"
    );
    assert!(
        frame.map(PixelFormat::Bgra8, &operation).is_ok(),
        "a retained frame supports frame-local work after its session closes"
    );
}

fn open_first(provider: &dyn CaptureProvider) -> Arc<dyn mado_pilot_capture::CaptureSession> {
    let operation = bounded();
    let targets = provider.discover(&operation).expect("discovery succeeds");
    let target = targets.first().expect("at least one target");
    provider
        .open(target.id(), &OpenRequest::new(), &operation)
        .expect("the discovered target opens")
}
