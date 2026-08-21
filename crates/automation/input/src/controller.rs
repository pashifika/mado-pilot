//! The provider and controller contracts an input Adapter implements.

use std::fmt::Debug;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::Duration;

use mado_pilot_core::{
    EngineId, InputCapability, InputDelivery, InputOperationKind, Lifecycle, Operation,
    OperationContext, PermissionKind, ProviderId, Result, TargetId,
};

use crate::descriptor::InputDescriptor;
use crate::fault::InputFault;
use crate::receipt::InputReceipt;
use crate::request::InputRequest;

/// How long a waiting sequence sleeps before re-checking its operation context.
///
/// A waiter cannot block for the whole remaining deadline, because the deadline is
/// measured on the operation's own clock and a test drives that clock by hand.
/// Waking periodically keeps one loop correct for both a real clock and a synthetic
/// one; the interval bounds how late an interruption is noticed, not how long a
/// sequence may take.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// Whether a caller can proceed without the input it asked for.
///
/// Opening a target establishes capture first. This says what happens when the
/// requested input capability turns out to be unavailable: a required capability
/// fails the open, and an optional one opens capture-only and reports what was
/// actually established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum InputRequirement {
    /// The session is useful without input. Capture opens either way.
    #[default]
    Optional,
    /// The session is not useful without input. An unavailable capability fails
    /// the open, and the capture session committed for it is released.
    Required,
}

impl InputRequirement {
    /// Reports whether an unavailable capability must fail the open.
    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self, InputRequirement::Required)
    }
}

/// What a caller asks for when establishing input on a target.
///
/// Required and preferred combinations are separate axes for the reason a capture
/// open request separates them: a required combination that cannot be established
/// fails, a preferred one falls back, and the accepted descriptor reports which
/// the caller actually got. Collapsing them would mean a caller either cannot say
/// "I need this" or cannot tell whether it got it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputOpenRequest {
    requirement: InputRequirement,
    required: Vec<(InputOperationKind, InputDelivery)>,
    preferred: Vec<(InputOperationKind, InputDelivery)>,
}

impl InputOpenRequest {
    /// Returns a request that requires nothing and prefers nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares whether the caller can proceed without input at all.
    #[must_use]
    pub const fn with_requirement(mut self, requirement: InputRequirement) -> Self {
        self.requirement = requirement;
        self
    }

    /// Requires one operation and route combination.
    #[must_use]
    pub fn requiring(mut self, kind: InputOperationKind, route: InputDelivery) -> Self {
        let pair = (kind, route);
        if !self.required.contains(&pair) {
            self.required.push(pair);
        }
        self
    }

    /// Prefers one operation and route combination.
    #[must_use]
    pub fn preferring(mut self, kind: InputOperationKind, route: InputDelivery) -> Self {
        let pair = (kind, route);
        if !self.preferred.contains(&pair) {
            self.preferred.push(pair);
        }
        self
    }

    /// Returns whether input is required for the session to open.
    #[must_use]
    pub const fn requirement(&self) -> InputRequirement {
        self.requirement
    }

    /// Returns the combinations that must be established.
    #[must_use]
    pub fn required(&self) -> &[(InputOperationKind, InputDelivery)] {
        &self.required
    }

    /// Returns the combinations the caller would like.
    #[must_use]
    pub fn preferred(&self) -> &[(InputOperationKind, InputDelivery)] {
        &self.preferred
    }

    /// Checks `capability` against what this request requires.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::UnsupportedCombination`] when a required pair cannot
    /// be attempted, and when input is required at all and none is available. An
    /// unknown pair is attemptable; a preferred pair that is absent is not an
    /// error, and the accepted descriptor reports what is there.
    pub fn check(&self, capability: InputCapability) -> Result<()> {
        if self.requirement.is_required() && !capability.is_available() {
            return Err(InputFault::UnsupportedCombination.into());
        }
        for (kind, route) in &self.required {
            if !capability.pair(*kind, *route).may_attempt() {
                return Err(InputFault::UnsupportedCombination.into());
            }
        }
        Ok(())
    }
}

/// Confirms that a capture provider and an input provider belong together.
///
/// Wiring calls this before it accepts a pair. A target identity is qualified by
/// the provider that issued it, so an input Adapter handed another provider's
/// target would be acting on an ordinal that means nothing to it — and, because
/// ordinals are per provider, one that quite possibly names a real target of its
/// own.
///
/// # Errors
///
/// Returns [`InputFault::ProviderMismatch`] when the two differ.
pub fn check_provider_pair(capture: ProviderId, input: ProviderId) -> Result<()> {
    if capture == input {
        Ok(())
    } else {
        Err(InputFault::ProviderMismatch.into())
    }
}

/// A source of input controllers.
///
/// Implemented by platform Adapters and by test doubles. A provider reports what a
/// target accepts and opens a controller for it; it never delivers input itself,
/// because delivery has to be serialized per target and a stateless provider has
/// nowhere to serialize it.
pub trait InputProvider: Debug + Send + Sync {
    /// Returns the provider that qualifies the target identities this accepts.
    fn provider(&self) -> ProviderId;

    /// Returns the authorization this provider's input ordinarily requires.
    ///
    /// `None` means the platform grants no separate input authorization, which is
    /// not a claim that input will succeed.
    fn permission(&self) -> Option<PermissionKind> {
        Some(PermissionKind::InputControl)
    }

    /// Describes what `target` accepts, without establishing anything.
    ///
    /// # Errors
    ///
    /// Returns an invalid-argument outcome for a target this provider did not
    /// issue, a target-lost outcome for one that no longer exists, and the
    /// operation's terminal outcome when cancellation or the deadline wins.
    fn describe(&self, target: TargetId, operation: &OperationContext) -> Result<InputDescriptor>;

    /// Opens a controller for `target`.
    ///
    /// # Errors
    ///
    /// Returns an unsupported outcome when a required combination cannot be
    /// established, an invalid-argument outcome for a foreign target, and the
    /// operation's terminal outcome when cancellation or the deadline wins.
    fn open(
        &self,
        target: TargetId,
        request: &InputOpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn InputController>>;

    /// Confirms that this provider issued `target`, for `engine`.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::ForeignTarget`] when either half does not match.
    fn accepts_target(&self, target: TargetId, engine: EngineId) -> Result<()> {
        target
            .check_issued_by(engine, self.provider())
            .map_err(|_| InputFault::ForeignTarget)?;
        Ok(())
    }
}

/// One open input controller, serializing the sequences sent to its target.
///
/// # Non-interleaving
///
/// A controller executes one sequence at a time. Two callers that press a modifier
/// and then a key would otherwise interleave into a keystroke neither of them
/// asked for. Waiting for the controller is governed by the caller's own operation
/// context and there is no internal queue: a sequence whose deadline passes while
/// it waits returns an unexecuted receipt, so pressure is reported to callers
/// instead of accumulating inside the Adapter. [`Admission`] implements that rule
/// once for every Adapter.
///
/// # Ordering
///
/// Waiting sequences are not ordered among themselves. Whichever waiter observes
/// the controller free next proceeds, and the operation deadline is what decides
/// who gives up. A queue would give ordering, at the price of a backlog that grows
/// with the caller's rate.
pub trait InputController: Debug + Send + Sync {
    /// Returns what this controller's target accepts.
    fn descriptor(&self) -> InputDescriptor;

    /// Executes `request`, returning exactly one receipt.
    ///
    /// An admitted sequence always produces a receipt, including when it fails
    /// part-way: events that may already have native effect cannot be taken back,
    /// so the receipt records how far the route got rather than application effect.
    ///
    /// The operation context bounds the sequence. It does **not** bound the
    /// releases that follow a partial failure: those run under the request's
    /// [`CleanupBudget`](crate::CleanupBudget), through the context that
    /// [`CleanupBudget::context`](crate::CleanupBudget::context) derives from this
    /// one. Cleanup usually runs *because* the operation was interrupted, so
    /// releasing under the interrupted context would decline to release pressed
    /// state at the one moment it matters.
    ///
    /// # Errors
    ///
    /// Returns an error only when no receipt can be produced — a request that
    /// fails admission, a closed controller, or an operation that was already
    /// interrupted. Once a sequence is admitted, a submission failure is reported
    /// inside the receipt and not as an error, because an error would discard its
    /// route-threshold accounting.
    fn execute(&self, request: &InputRequest, operation: &OperationContext)
    -> Result<InputReceipt>;

    /// Stops admitting sequences and drains the one in flight.
    ///
    /// Idempotent. A retried close neither submits an event nor repeats cleanup.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the deadline
    /// wins before the drain finishes. The controller then stays closing, and a
    /// later close continues.
    fn close(&self, operation: &OperationContext) -> Result<()>;

    /// Returns where the controller is in its lifecycle.
    fn lifecycle(&self) -> Lifecycle;

    /// Reports whether the controller still accepts sequences.
    fn is_open(&self) -> bool {
        self.lifecycle() == Lifecycle::Open
    }

    /// Reports whether the controller has finished closing.
    fn is_closed(&self) -> bool {
        self.lifecycle() == Lifecycle::Closed
    }
}

/// Serializes one controller's sequences, without a queue.
///
/// Adapters hold one of these and take a guard around each sequence. The rule it
/// implements is small but easy to get subtly wrong — a waiter must consult its own
/// clock with no lock held, an expired waiter must not leave the controller marked
/// busy, and close must not strand a sequence — so it is implemented once here
/// rather than in each Adapter.
#[derive(Debug)]
pub struct Admission {
    inner: Mutex<AdmissionState>,
    released: Condvar,
}

#[derive(Debug)]
struct AdmissionState {
    busy: bool,
    owner: Option<ThreadId>,
    waiters: usize,
    pending_admissions: usize,
    lifecycle: Lifecycle,
    entering: Vec<ThreadId>,
}

impl Admission {
    /// Returns an admission gate that is open and idle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(AdmissionState {
                busy: false,
                owner: None,
                waiters: 0,
                pending_admissions: 0,
                lifecycle: Lifecycle::Open,
                entering: Vec::new(),
            }),
            released: Condvar::new(),
        }
    }

    /// Returns where the controller is in its lifecycle.
    #[must_use]
    pub fn lifecycle(&self) -> Lifecycle {
        self.lock().lifecycle
    }

    /// Reports whether a sequence is executing.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.lock().busy
    }

    /// Waits until the controller is free, then claims it.
    ///
    /// The returned guard releases the controller when it is dropped, including when
    /// the sequence it guarded failed part-way.
    ///
    /// # Errors
    ///
    /// Returns [`InputFault::ControllerClosed`] once close has begun, and the
    /// operation's terminal outcome when cancellation or the deadline wins while
    /// waiting. Neither leaves the controller claimed.
    /// Synchronous reentry from caller-owned operation code on the thread
    /// acquiring an admission is rejected as [`InputFault::SubmissionFailed`]
    /// before that operation state is consulted again. The guard remains
    /// transferable; like other transferable lock guards, moving it does not
    /// make arbitrary recursive acquisition on its destination thread detectable.
    pub fn admit(&self, operation: &OperationContext) -> Result<AdmissionGuard<'_>> {
        let thread = thread::current().id();
        {
            let mut state = self.lock();
            if state.owner == Some(thread) || state.entering.contains(&thread) {
                return Err(InputFault::SubmissionFailed.into());
            }
            if state.lifecycle != Lifecycle::Open {
                return Err(InputFault::ControllerClosed.into());
            }
            state.entering.push(thread);
            state.pending_admissions += 1;
        }
        let mut entry = AdmissionEntry {
            admission: self,
            thread,
            registered: true,
            blocks_close: true,
            _not_send: PhantomData,
        };
        let mut attempt = Operation::admit(operation)?;
        loop {
            {
                let mut state = self.lock();
                if state.lifecycle != Lifecycle::Open {
                    return Err(InputFault::ControllerClosed.into());
                }
                if !state.busy {
                    state.busy = true;
                    state.owner = Some(thread);
                    entry.unregister(&mut state);
                    drop(state);
                    // The guard and its acquisition-thread marker exist before
                    // the final arbitration, so a custom clock reentering from
                    // `commit` is refused and an operation that expired releases
                    // the controller rather than leaving it claimed.
                    return Ok(attempt.commit(AdmissionGuard {
                        admission: self,
                        owner: thread,
                    })?);
                }
                state.waiters += 1;
                let (mut state, _) = self
                    .released
                    .wait_timeout(state, POLL_INTERVAL)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.waiters -= 1;
                if state.waiters == 0 {
                    drop(state);
                    // A drain may be waiting for the last waiter to leave.
                    self.released.notify_all();
                }
            }
            // The operation context is consulted with no lock held: its clock and
            // cancellation token are the caller's own code.
            attempt.checkpoint()?;
        }
    }

    /// Stops admitting new sequences and wakes every waiter.
    ///
    /// Idempotent, and never moves a closed controller backwards.
    pub fn begin_close(&self) {
        {
            let mut state = self.lock();
            if state.lifecycle == Lifecycle::Open {
                state.lifecycle = Lifecycle::Closing;
            }
        }
        self.released.notify_all();
    }

    /// Waits for the sequence in flight, its waiters, and pre-admission caller
    /// callbacks to unwind, then closes.
    ///
    /// # Errors
    ///
    /// Returns the operation's terminal outcome when cancellation or the deadline
    /// wins first. The controller then stays [`Lifecycle::Closing`], so a later
    /// close continues the drain rather than restarting it.
    /// Synchronous reentry from caller-owned operation code on an active
    /// admission's acquisition thread is rejected as
    /// [`InputFault::SubmissionFailed`] before close begins or that operation
    /// state is consulted again. Moving the transferable guard does not extend
    /// this protection to arbitrary recursive use on its destination thread.
    pub fn drain(&self, operation: &OperationContext) -> Result<()> {
        let thread = thread::current().id();
        {
            let mut state = self.lock();
            if state.owner == Some(thread) || state.entering.contains(&thread) {
                return Err(InputFault::SubmissionFailed.into());
            }
            state.entering.push(thread);
        }
        let _entry = AdmissionEntry {
            admission: self,
            thread,
            registered: true,
            blocks_close: false,
            _not_send: PhantomData,
        };
        self.begin_close();
        let mut attempt = Operation::admit(operation)?;
        loop {
            {
                let state = self.lock();
                if state.lifecycle == Lifecycle::Closed {
                    drop(state);
                    return Ok(attempt.commit(())?);
                }
                if !state.busy && state.waiters == 0 && state.pending_admissions == 0 {
                    drop(state);
                    // The caller's clock is consulted by commit, so the final
                    // arbitration happens with no lock held and before the
                    // irreversible transition.
                    attempt.commit(())?;
                    let mut state = self.lock();
                    if state.lifecycle != Lifecycle::Closed {
                        debug_assert_eq!(state.lifecycle, Lifecycle::Closing);
                        state.lifecycle = Lifecycle::Closed;
                    }
                    return Ok(());
                }
                let _unused = self
                    .released
                    .wait_timeout(state, POLL_INTERVAL)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            attempt.checkpoint()?;
        }
    }

    fn lock(&self) -> MutexGuard<'_, AdmissionState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for Admission {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct AdmissionEntry<'admission> {
    admission: &'admission Admission,
    thread: ThreadId,
    registered: bool,
    blocks_close: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl AdmissionEntry<'_> {
    fn unregister(&mut self, state: &mut AdmissionState) -> bool {
        let index = state
            .entering
            .iter()
            .position(|thread| *thread == self.thread)
            .expect("a live admission entry remains registered");
        state.entering.swap_remove(index);
        if self.blocks_close {
            state.pending_admissions -= 1;
        }
        self.registered = false;
        self.blocks_close && state.pending_admissions == 0
    }
}

impl Drop for AdmissionEntry<'_> {
    fn drop(&mut self) {
        if self.registered {
            let notify = {
                let mut state = self.admission.lock();
                self.unregister(&mut state)
            };
            if notify {
                self.admission.released.notify_all();
            }
        }
    }
}

/// The right to execute one sequence on one controller.
#[derive(Debug)]
pub struct AdmissionGuard<'admission> {
    admission: &'admission Admission,
    owner: ThreadId,
}

impl Drop for AdmissionGuard<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.admission.lock();
            debug_assert_eq!(state.owner, Some(self.owner));
            state.busy = false;
            state.owner = None;
        }
        self.admission.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    use super::{
        Admission, AdmissionGuard, InputOpenRequest, InputRequirement, check_provider_pair,
    };
    use mado_pilot_core::{
        CancellationToken, CapabilitySupport, Clock, InputCapability, InputDelivery,
        InputOperationKind, Lifecycle, MonotonicInstant, OperationContext, ProviderId, Status,
        SubmissionEvidence,
    };

    const WINDOWS: ProviderId = ProviderId::new("windows");

    #[derive(Debug, Clone, Copy)]
    enum Reentry {
        Admit,
        Drain,
    }

    #[derive(Debug)]
    struct PanicClock;

    impl Clock for PanicClock {
        fn now(&self) -> MonotonicInstant {
            panic!("a reentrant admission consulted caller-owned operation state");
        }
    }

    #[derive(Debug)]
    struct ReentrantClock {
        admission: Arc<Admission>,
        reentry: Reentry,
        calls: AtomicUsize,
        rejected: AtomicBool,
        observed: Mutex<Option<Status>>,
    }

    impl ReentrantClock {
        fn new(admission: Arc<Admission>, reentry: Reentry) -> Self {
            Self {
                admission,
                reentry,
                calls: AtomicUsize::new(0),
                rejected: AtomicBool::new(false),
                observed: Mutex::new(None),
            }
        }
    }

    impl Clock for ReentrantClock {
        fn now(&self) -> MonotonicInstant {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                let nested = OperationContext::new()
                    .with_clock(Arc::new(PanicClock))
                    .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
                let status = match self.reentry {
                    Reentry::Admit => self
                        .admission
                        .admit(&nested)
                        .err()
                        .map(|error| error.status()),
                    Reentry::Drain => self
                        .admission
                        .drain(&nested)
                        .err()
                        .map(|error| error.status()),
                };
                self.rejected
                    .store(status == Some(Status::InputFailed), Ordering::Release);
                *self
                    .observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
            }
            MonotonicInstant::from_origin(Duration::ZERO)
        }
    }

    #[derive(Debug)]
    struct BlockingClock {
        blocked: AtomicBool,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Clock for BlockingClock {
        fn now(&self) -> MonotonicInstant {
            if !self.blocked.swap(true, Ordering::AcqRel) {
                self.entered.send(()).expect("report blocked clock");
                self.release
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .recv()
                    .expect("release blocked clock");
            }
            MonotonicInstant::from_origin(Duration::ZERO)
        }
    }
    const REPLAY: ProviderId = ProviderId::new("replay");
    #[test]
    fn admission_guard_remains_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<AdmissionGuard<'static>>();
    }

    #[test]
    fn providers_must_be_the_same_to_be_wired_together() {
        assert!(check_provider_pair(WINDOWS, WINDOWS).is_ok());
        assert_eq!(
            check_provider_pair(WINDOWS, REPLAY)
                .expect_err("a mismatch is refused")
                .status(),
            Status::InvalidArgument
        );
    }

    #[test]
    fn optional_input_accepts_a_capture_only_target() {
        let request = InputOpenRequest::new();

        assert_eq!(request.requirement(), InputRequirement::Optional);
        assert!(request.check(InputCapability::none()).is_ok());
    }

    #[test]
    fn required_input_refuses_a_capture_only_target() {
        let request = InputOpenRequest::new().with_requirement(InputRequirement::Required);

        assert_eq!(
            request
                .check(InputCapability::none())
                .expect_err("required input is not available")
                .status(),
            Status::Unsupported
        );
    }

    #[test]
    fn a_required_combination_must_be_present_and_a_preferred_one_need_not_be() {
        let capability = InputCapability::none().with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Unknown,
            SubmissionEvidence::SystemInputAdmission,
        );
        let request = InputOpenRequest::new()
            .requiring(InputOperationKind::Keyboard, InputDelivery::System)
            .preferring(InputOperationKind::Pointer, InputDelivery::WindowMessage);

        assert!(
            request.check(capability).is_ok(),
            "unknown remains attemptable"
        );
        assert_eq!(request.required().len(), 1);
        assert_eq!(request.preferred().len(), 1);

        let stricter = request.requiring(InputOperationKind::Pointer, InputDelivery::WindowMessage);
        assert_eq!(
            stricter
                .check(capability)
                .expect_err("the added requirement is absent")
                .status(),
            Status::Unsupported
        );
    }

    #[test]
    fn a_repeated_combination_is_recorded_once() {
        let request = InputOpenRequest::new()
            .requiring(InputOperationKind::Text, InputDelivery::System)
            .requiring(InputOperationKind::Text, InputDelivery::System);

        assert_eq!(request.required().len(), 1);
    }

    #[test]
    fn one_sequence_at_a_time_holds_the_controller() {
        let admission = Admission::new();
        let context = OperationContext::new();

        let guard = admission.admit(&context).expect("free");
        assert!(admission.is_busy());
        drop(guard);
        assert!(!admission.is_busy());
        admission.admit(&context).expect("free again");
    }

    #[test]
    fn a_waiting_sequence_proceeds_when_the_controller_is_released() {
        let admission = Arc::new(Admission::new());
        let (entered_tx, entered_rx) = mpsc::channel();

        thread::scope(|scope| {
            let gate = Arc::clone(&admission);
            let worker = scope.spawn(move || {
                let guard = gate.admit(&OperationContext::new()).expect("free");
                entered_tx.send(()).expect("entered");
                thread::sleep(Duration::from_millis(10));
                drop(guard);
            });
            entered_rx.recv().expect("worker entered");

            let second = admission
                .admit(&OperationContext::new())
                .expect("woken by the release");
            worker.join().expect("releaser finished");

            assert!(admission.is_busy(), "the woken sequence holds it now");
            drop(second);
        });

        assert!(!admission.is_busy());
    }

    #[test]
    fn a_caller_clock_reentering_admission_is_rejected_without_waiting() {
        let admission = Arc::new(Admission::new());
        let clock = Arc::new(ReentrantClock::new(Arc::clone(&admission), Reentry::Admit));
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        let guard = admission.admit(&context).expect("outer admission succeeds");

        assert!(clock.rejected.load(Ordering::Acquire));
        assert_eq!(
            *clock
                .observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Some(Status::InputFailed)
        );
        assert_eq!(admission.lifecycle(), Lifecycle::Open);
        drop(guard);
    }

    #[test]
    fn a_caller_clock_reentering_drain_is_rejected_without_closing_or_waiting() {
        let admission = Arc::new(Admission::new());
        let clock = Arc::new(ReentrantClock::new(Arc::clone(&admission), Reentry::Drain));
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        let guard = admission.admit(&context).expect("outer admission succeeds");

        assert!(clock.rejected.load(Ordering::Acquire));
        assert_eq!(
            *clock
                .observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Some(Status::InputFailed)
        );
        assert_eq!(admission.lifecycle(), Lifecycle::Open);
        drop(guard);
    }

    #[test]
    fn an_in_flight_driver_checkpoint_rejects_same_thread_reentry() {
        for reentry in [Reentry::Admit, Reentry::Drain] {
            let admission = Arc::new(Admission::new());
            let guard = admission
                .admit(&OperationContext::new())
                .expect("outer admission succeeds");
            let clock = Arc::new(ReentrantClock::new(Arc::clone(&admission), reentry));
            let context = OperationContext::new()
                .with_clock(clock.clone())
                .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

            assert_eq!(context.interruption(), None);
            assert!(clock.rejected.load(Ordering::Acquire));
            assert_eq!(
                *clock
                    .observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                Some(Status::InputFailed)
            );
            assert_eq!(admission.lifecycle(), Lifecycle::Open);
            drop(guard);
        }
    }

    #[test]
    fn a_sequence_whose_deadline_passes_while_waiting_never_runs() {
        let admission = Arc::new(Admission::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let expiring = OperationContext::new()
            .with_timeout(Duration::from_millis(20))
            .expect("representable");

        thread::scope(|scope| {
            let gate = Arc::clone(&admission);
            let holder = scope.spawn(move || {
                let guard = gate.admit(&OperationContext::new()).expect("free");
                entered_tx.send(()).expect("entered");
                release_rx.recv().expect("release");
                drop(guard);
            });
            entered_rx.recv().expect("holder entered");

            let error = admission
                .admit(&expiring)
                .expect_err("the controller stayed busy");
            assert_eq!(error.status(), Status::DeadlineExceeded);
            assert!(
                admission.is_busy(),
                "the expired waiter did not claim the controller"
            );
            release_tx.send(()).expect("release holder");
            holder.join().expect("holder finished");
        });

        assert!(!admission.is_busy());
    }

    #[test]
    fn a_cancelled_sequence_does_not_claim_the_controller() {
        let admission = Admission::new();
        let token = CancellationToken::new();
        token.cancel();
        let cancelled = OperationContext::new().with_cancellation(token);

        let error = admission.admit(&cancelled).expect_err("cancelled");

        assert_eq!(error.status(), Status::Cancelled);
        assert!(!admission.is_busy());
    }

    #[test]
    fn closing_refuses_new_sequences_and_wakes_waiters() {
        let admission = Arc::new(Admission::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        thread::scope(|scope| {
            let gate = Arc::clone(&admission);
            let holder = scope.spawn(move || {
                let guard = gate.admit(&OperationContext::new()).expect("free");
                entered_tx.send(()).expect("entered");
                release_rx.recv().expect("release");
                drop(guard);
            });
            entered_rx.recv().expect("holder entered");

            let closer = Arc::clone(&admission);
            let close = scope.spawn(move || {
                thread::sleep(Duration::from_millis(10));
                closer.begin_close();
            });
            let error = admission
                .admit(&OperationContext::new())
                .expect_err("closed while waiting");
            close.join().expect("closer finished");

            assert_eq!(error.status(), Status::Closed);
            assert_eq!(admission.lifecycle(), Lifecycle::Closing);
            release_tx.send(()).expect("release holder");
            holder.join().expect("holder finished");
        });
    }

    #[test]
    fn close_is_idempotent() {
        let admission = Admission::new();
        let context = OperationContext::new();

        admission.drain(&context).expect("drained");
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
        admission.drain(&context).expect("already drained");
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn caller_clock_reentry_from_drain_is_rejected_without_recursing() {
        let admission = Arc::new(Admission::new());
        let clock = Arc::new(ReentrantClock::new(Arc::clone(&admission), Reentry::Drain));
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        admission.drain(&context).expect("outer drain completes");

        assert!(clock.rejected.load(Ordering::Acquire));
        assert_eq!(
            *clock
                .observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Some(Status::InputFailed)
        );
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn drain_waits_for_pre_admission_caller_code_to_return() {
        let admission = Arc::new(Admission::new());
        let (clock_entered_tx, clock_entered_rx) = mpsc::channel();
        let (release_clock_tx, release_clock_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let clock = Arc::new(BlockingClock {
            blocked: AtomicBool::new(false),
            entered: clock_entered_tx,
            release: Mutex::new(release_clock_rx),
        });
        let context = OperationContext::new()
            .with_clock(clock)
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));

        thread::scope(|scope| {
            let worker_gate = Arc::clone(&admission);
            let worker = scope.spawn(move || match worker_gate.admit(&context) {
                Ok(guard) => {
                    drop(guard);
                    None
                }
                Err(error) => Some(error.status()),
            });
            clock_entered_rx
                .recv()
                .expect("admission entered caller clock");

            let close_gate = Arc::clone(&admission);
            let closer = scope.spawn(move || {
                let result = close_gate.drain(&OperationContext::new());
                closed_tx.send(result).expect("report close result");
            });
            assert!(
                closed_rx.recv_timeout(Duration::from_millis(50)).is_err(),
                "close cannot finish while pre-admission caller code is still executing"
            );

            release_clock_tx.send(()).expect("release caller clock");
            let status = worker
                .join()
                .expect("admission worker returns")
                .expect("closing controller refuses the pending admission");
            assert_eq!(status, Status::Closed);
            closed_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("close finishes after admission callback returns")
                .expect("drain succeeds");
            closer.join().expect("closer returns");
        });
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn a_drain_waits_for_the_sequence_in_flight() {
        let admission = Arc::new(Admission::new());
        let (entered_tx, entered_rx) = mpsc::channel();

        let lifecycle_during = thread::scope(|scope| {
            let gate = Arc::clone(&admission);
            let worker = scope.spawn(move || {
                let guard = gate.admit(&OperationContext::new()).expect("free");
                entered_tx.send(()).expect("entered");
                thread::sleep(Duration::from_millis(10));
                let seen = gate.lifecycle();
                drop(guard);
                seen
            });
            entered_rx.recv().expect("worker entered");

            admission
                .drain(&OperationContext::new())
                .expect("drained after the sequence finished");
            worker.join().expect("worker finished")
        });

        assert_eq!(
            lifecycle_during,
            Lifecycle::Closing,
            "close stopped admission before the sequence finished"
        );
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
    }

    #[test]
    fn a_close_that_expires_leaves_the_controller_closing() {
        let admission = Arc::new(Admission::new());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let expiring = OperationContext::new()
            .with_timeout(Duration::from_millis(20))
            .expect("representable");

        thread::scope(|scope| {
            let gate = Arc::clone(&admission);
            let holder = scope.spawn(move || {
                let guard = gate.admit(&OperationContext::new()).expect("free");
                entered_tx.send(()).expect("entered");
                release_rx.recv().expect("release");
                drop(guard);
            });
            entered_rx.recv().expect("holder entered");

            let error = admission
                .drain(&expiring)
                .expect_err("the sequence is stuck");
            assert_eq!(error.status(), Status::DeadlineExceeded);
            assert_eq!(admission.lifecycle(), Lifecycle::Closing);
            release_tx.send(()).expect("release holder");
            holder.join().expect("holder finished");
        });

        admission
            .drain(&OperationContext::new())
            .expect("a later close continues the drain");
        assert_eq!(admission.lifecycle(), Lifecycle::Closed);
    }
}
