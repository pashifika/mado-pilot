//! The measurement scaffolding every in-process benchmark target shares.
//!
//! The deterministic Rust workflow, C boundary, and diagnostic-overhead
//! benchmarks all emit the profile format `docs/performance.md` defines. They
//! measure different things and must emit the *same* shape, because a committed
//! profile is read by whoever compares two runs and a second copy of the printer
//! is a second thing to keep in step with the format document.
//!
//! What is here is everything that is not a workload: the sampling loop, the
//! allocation accounting, the host arguments, the report, and the hard budgets.
//! What each benchmark keeps for itself is its fixtures, its workloads, and its
//! oracles.
//!
//! # Which budgets are enforced here
//!
//! A `hard` budget is a structural property that holds on any host, so the
//! harness enforces it: [`enforce_hard_budgets`] is called by every in-process
//! benchmark target on both of the paths they run. An `absolute` or `relative`
//! budget is a per-target regression ceiling measured on named hardware, so
//! only a run on that hardware can evaluate it; those stay with the operator and
//! committed profile for the matching release target.
//!
//! # Why the counters live in a library
//!
//! A `#[global_allocator]` is per binary, so each benchmark declares its own
//! static. [`Accounting`] is the implementation they all point at, and the
//! counters are its statics, which is what lets [`measure`] read them without
//! any benchmark passing them in.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_PROCESS_POLL: Duration = Duration::from_millis(5);
const CHILD_PROCESS_TERMINATE_WAIT: Duration = Duration::from_secs(1);
const CHILD_PIPE_DRAIN_WAIT: Duration = Duration::from_millis(100);

/// Captured output from one benchmark child whose lifetime and output were
/// bounded by [`bounded_child_output`].
#[derive(Debug)]
pub struct BoundedChildOutput {
    /// The process status observed before this bounded result, or `None` when
    /// the dedicated reaper still owns an unreaped child.
    pub status: Option<ExitStatus>,
    /// The retained stdout prefix, never longer than the requested byte cap.
    pub stdout: Vec<u8>,
    /// The retained stderr prefix, never longer than the requested byte cap.
    pub stderr: Vec<u8>,
    /// Whether the child exited before its deadline and both streams completed
    /// without exceeding the byte cap.
    pub within_bounds: bool,
}

struct CappedPipe {
    bytes: Vec<u8>,
    overflowed: bool,
    complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipeStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipeReaderEvent {
    Eof(PipeStream),
    Joined(PipeStream),
}

struct PipeReader {
    handle: JoinHandle<CappedPipe>,
    stream: PipeStream,
    events: Option<mpsc::Sender<PipeReaderEvent>>,
}

impl PipeReader {
    fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    fn join(self) -> thread::Result<CappedPipe> {
        let result = self.handle.join();
        if let Some(events) = self.events {
            let _sent = events.send(PipeReaderEvent::Joined(self.stream));
        }
        result
    }
}

fn read_capped_pipe(mut pipe: impl Read, max_bytes: usize) -> CappedPipe {
    let mut bytes = Vec::with_capacity(max_bytes);
    let mut chunk = [0u8; 4_096];
    let mut overflowed = false;
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => {
                return CappedPipe {
                    bytes,
                    overflowed,
                    complete: true,
                };
            }
            Ok(count) => {
                let remaining = max_bytes.saturating_sub(bytes.len());
                let retained = remaining.min(count);
                bytes.extend_from_slice(&chunk[..retained]);
                overflowed |= retained != count;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                return CappedPipe {
                    bytes,
                    overflowed,
                    complete: false,
                };
            }
        }
    }
}

fn spawn_capped_pipe_reader(
    pipe: impl Read + Send + 'static,
    max_bytes: usize,
    stream: PipeStream,
    events: Option<mpsc::Sender<PipeReaderEvent>>,
) -> std::io::Result<PipeReader> {
    let eof_events = events.clone();
    let handle = thread::Builder::new().spawn(move || {
        let result = read_capped_pipe(pipe, max_bytes);
        if result.complete
            && let Some(events) = eof_events
        {
            let _sent = events.send(PipeReaderEvent::Eof(stream));
        }
        result
    })?;
    Ok(PipeReader {
        handle,
        stream,
        events,
    })
}

#[derive(Default)]
struct ChildExitObservation {
    status: Option<ExitStatus>,
    exited_unreaped: bool,
}

impl ChildExitObservation {
    fn exited(&self) -> bool {
        self.status.is_some() || self.exited_unreaped
    }
}

fn observe_child_exit(child: &mut Child) -> ChildExitObservation {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(exited_unreaped) = child_exited_without_reaping(child.id()) {
        return ChildExitObservation {
            status: None,
            exited_unreaped,
        };
    }

    ChildExitObservation {
        status: child.try_wait().ok().flatten(),
        exited_unreaped: false,
    }
}

fn wait_for_child_exit(child: &mut Child, deadline: Instant) -> ChildExitObservation {
    while Instant::now() < deadline {
        let observation = observe_child_exit(child);
        if observation.exited() {
            return observation;
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(CHILD_PROCESS_POLL),
        );
    }
    observe_child_exit(child)
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn kill_process_group(process: i32, signal: i32) -> i32;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn waitid(id_type: i32, id: u32, information: *mut WaitSignalInfo, options: i32) -> i32;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[repr(C, align(16))]
struct WaitSignalInfo([u8; 128]);

#[cfg(target_os = "macos")]
const WAIT_SIGNAL_PID_OFFSET: usize = 12;
#[cfg(target_os = "linux")]
const WAIT_SIGNAL_PID_OFFSET: usize = 16;
#[cfg(target_os = "macos")]
const WAIT_LEAVE_WAITABLE: i32 = 0x0000_0020;
#[cfg(target_os = "linux")]
const WAIT_LEAVE_WAITABLE: i32 = 0x0100_0000;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn child_exited_without_reaping(process_id: u32) -> Option<bool> {
    const P_PID: i32 = 1;
    const WEXITED: i32 = 0x0000_0004;
    const WNOHANG: i32 = 0x0000_0001;

    let mut information = WaitSignalInfo([0; 128]);
    // SAFETY: `information` has the platform `siginfo_t` size/alignment and is
    // writable for the full call. `WNOWAIT` preserves the child as waitable,
    // keeping its process-group identity reserved until explicit reaping.
    let result = unsafe {
        waitid(
            P_PID,
            process_id,
            &raw mut information,
            WEXITED | WNOHANG | WAIT_LEAVE_WAITABLE,
        )
    };
    if result != 0 {
        return None;
    }
    let pid = i32::from_ne_bytes([
        information.0[WAIT_SIGNAL_PID_OFFSET],
        information.0[WAIT_SIGNAL_PID_OFFSET + 1],
        information.0[WAIT_SIGNAL_PID_OFFSET + 2],
        information.0[WAIT_SIGNAL_PID_OFFSET + 3],
    ]);
    Some(pid != 0)
}

#[cfg(unix)]
struct PreparedChildContainment;

#[cfg(unix)]
struct ChildContainment {
    process_group: Option<i32>,
}

#[cfg(unix)]
impl PreparedChildContainment {
    fn prepare(command: &mut Command) -> Option<Self> {
        use std::os::unix::process::CommandExt as _;

        command.process_group(0);
        Some(Self)
    }

    fn attach(self, child: &Child) -> (ChildContainment, bool) {
        let process_group = i32::try_from(child.id()).ok().filter(|id| *id > 0);
        let contained = process_group.is_some();
        (ChildContainment { process_group }, contained)
    }
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Child;
    use std::ptr;

    pub(super) const CREATE_SUSPENDED: u32 = 0x0000_0004;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const THREAD_SUSPEND_RESUME: u32 = 0x0000_0002;
    const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
    #[cfg(test)]
    const SYNCHRONIZE: u32 = 0x0010_0000;
    #[cfg(test)]
    const WAIT_TIMEOUT: u32 = 258;

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "CreateJobObjectW"]
        fn create_job_object(job_attributes: *const c_void, name: *const u16) -> *mut c_void;
        #[link_name = "SetInformationJobObject"]
        fn set_information_job_object(
            job: *mut c_void,
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
        #[link_name = "AssignProcessToJobObject"]
        fn assign_process_to_job_object(job: *mut c_void, process: *mut c_void) -> i32;
        #[link_name = "TerminateJobObject"]
        fn terminate_job_object(job: *mut c_void, exit_code: u32) -> i32;
        #[link_name = "CreateToolhelp32Snapshot"]
        fn create_toolhelp32_snapshot(flags: u32, process_id: u32) -> *mut c_void;
        #[link_name = "Thread32First"]
        fn thread32_first(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        #[link_name = "Thread32Next"]
        fn thread32_next(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        #[link_name = "OpenThread"]
        fn open_thread(desired_access: u32, inherit_handle: i32, thread_id: u32) -> *mut c_void;
        #[link_name = "ResumeThread"]
        fn resume_thread(thread: *mut c_void) -> u32;
        #[cfg(test)]
        #[link_name = "OpenProcess"]
        fn open_process(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        #[cfg(test)]
        #[link_name = "WaitForSingleObject"]
        fn wait_for_single_object(handle: *mut c_void, milliseconds: u32) -> u32;
    }

    pub(super) struct Job {
        handle: OwnedHandle,
    }

    impl Job {
        pub(super) fn new() -> Option<Self> {
            // SAFETY: both nullable inputs follow `CreateJobObjectW`'s contract.
            let raw_handle = unsafe { create_job_object(ptr::null(), ptr::null()) };
            if raw_handle.is_null() {
                return None;
            }
            // SAFETY: a non-null successful `CreateJobObjectW` result is one
            // owned Windows handle, transferred exactly once to `OwnedHandle`.
            let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle) };
            let information = JobObjectExtendedLimitInformation {
                basic_limit_information: JobObjectBasicLimitInformation {
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..JobObjectBasicLimitInformation::default()
                },
                ..JobObjectExtendedLimitInformation::default()
            };
            let information_length =
                u32::try_from(size_of::<JobObjectExtendedLimitInformation>()).ok()?;
            // SAFETY: `handle` is a live job handle and `information` is a
            // correctly laid out value that remains alive for the full call.
            let configured = unsafe {
                set_information_job_object(
                    handle.as_raw_handle(),
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    ptr::addr_of!(information).cast(),
                    information_length,
                )
            };
            (configured != 0).then_some(Self { handle })
        }

        pub(super) fn assign(&self, child: &Child) -> bool {
            // SAFETY: both raw handles are borrowed from live owning values for
            // the duration of this call.
            unsafe {
                assign_process_to_job_object(self.handle.as_raw_handle(), child.as_raw_handle())
                    != 0
            }
        }

        pub(super) fn terminate(&self) {
            // SAFETY: the raw handle is borrowed from this live `OwnedHandle`.
            let _terminated = unsafe { terminate_job_object(self.handle.as_raw_handle(), 1) };
        }
    }

    pub(super) fn resume_process(process_id: u32) -> bool {
        // SAFETY: the flags and ignored process identifier follow the snapshot
        // API contract and the returned handle is validated before use.
        let raw_snapshot = unsafe { create_toolhelp32_snapshot(TH32CS_SNAPTHREAD, 0) };
        let invalid_handle = ptr::without_provenance_mut::<c_void>(usize::MAX);
        if raw_snapshot == invalid_handle {
            return false;
        }
        // SAFETY: a successful snapshot result is one owned Windows handle,
        // transferred exactly once to `OwnedHandle`.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw_snapshot) };
        let Ok(entry_size) = u32::try_from(size_of::<ThreadEntry>()) else {
            return false;
        };
        let mut entry = ThreadEntry {
            size: entry_size,
            ..ThreadEntry::default()
        };
        // SAFETY: `snapshot` is live and `entry` points to writable storage
        // whose leading size field declares its complete layout.
        let mut has_entry =
            unsafe { thread32_first(snapshot.as_raw_handle(), ptr::addr_of_mut!(entry)) };
        while has_entry != 0 {
            if entry.owner_process_id == process_id {
                // SAFETY: the identifier came from the live system snapshot;
                // a null result is rejected before ownership is constructed.
                let raw_thread = unsafe { open_thread(THREAD_SUSPEND_RESUME, 0, entry.thread_id) };
                if !raw_thread.is_null() {
                    // SAFETY: a non-null successful `OpenThread` result is one
                    // owned handle, transferred exactly once to `OwnedHandle`.
                    let thread = unsafe { OwnedHandle::from_raw_handle(raw_thread) };
                    // SAFETY: `thread` is a live thread handle opened with the
                    // suspend/resume right required by `ResumeThread`.
                    return unsafe { resume_thread(thread.as_raw_handle()) } != u32::MAX;
                }
            }
            // SAFETY: the same live snapshot and initialized writable entry
            // satisfy `Thread32Next` for every iteration.
            has_entry =
                unsafe { thread32_next(snapshot.as_raw_handle(), ptr::addr_of_mut!(entry)) };
        }
        false
    }

    #[cfg(test)]
    pub(super) fn process_is_live(process_id: u32) -> bool {
        // SAFETY: the numeric PID is untrusted but valid input to `OpenProcess`;
        // a null failure result is rejected before constructing ownership.
        let raw_process = unsafe { open_process(SYNCHRONIZE, 0, process_id) };
        if raw_process.is_null() {
            return false;
        }
        // SAFETY: a non-null successful `OpenProcess` result is one owned
        // handle, transferred exactly once to `OwnedHandle`.
        let process = unsafe { OwnedHandle::from_raw_handle(raw_process) };
        // SAFETY: `process` is a live synchronizable process handle and a zero
        // timeout performs only a nonblocking state query.
        unsafe { wait_for_single_object(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
    }
}

#[cfg(windows)]
struct PreparedChildContainment {
    job: windows_job::Job,
}

#[cfg(windows)]
struct ChildContainment {
    job: Option<windows_job::Job>,
}

#[cfg(windows)]
impl PreparedChildContainment {
    fn prepare(command: &mut Command) -> Option<Self> {
        use std::os::windows::process::CommandExt as _;

        command.creation_flags(windows_job::CREATE_SUSPENDED);
        Some(Self {
            job: windows_job::Job::new()?,
        })
    }

    fn attach(self, child: &Child) -> (ChildContainment, bool) {
        let contained = self.job.assign(child);
        let job = contained.then_some(self.job);
        (ChildContainment { job }, contained)
    }
}

#[cfg(not(any(unix, windows)))]
struct PreparedChildContainment;

#[cfg(not(any(unix, windows)))]
struct ChildContainment;

#[cfg(not(any(unix, windows)))]
impl PreparedChildContainment {
    fn prepare(_command: &mut Command) -> Option<Self> {
        Some(Self)
    }

    fn attach(self, _child: &Child) -> (ChildContainment, bool) {
        (ChildContainment, true)
    }
}

impl ChildContainment {
    fn activate(&self, child: &Child) -> bool {
        #[cfg(windows)]
        {
            self.job.is_some() && windows_job::resume_process(child.id())
        }

        #[cfg(not(windows))]
        {
            let _child = child;
            true
        }
    }

    fn terminate(&mut self, child: Option<&mut Child>) {
        #[cfg(unix)]
        if let Some(process_group) = self.process_group {
            // SAFETY: `process_group` is the positive identifier returned for
            // the child configured as its own group leader. A negative PID asks
            // `kill(2)` to signal that group and `SIGKILL` has no payload.
            let _terminated = unsafe { kill_process_group(-process_group, 9) };
        }

        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }

        if let Some(child) = child {
            let _terminated = child.kill();
        }
    }
}

#[cfg(all(test, unix))]
fn process_is_live(process_id: u32) -> bool {
    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    // SAFETY: signal zero performs only an existence/permission query for the
    // positive PID and carries no payload.
    unsafe { kill_process_group(process_id, 0) == 0 }
}

#[cfg(all(test, windows))]
fn process_is_live(process_id: u32) -> bool {
    windows_job::process_is_live(process_id)
}

#[cfg(all(test, not(any(unix, windows))))]
fn process_is_live(_process_id: u32) -> bool {
    false
}

struct PendingChildCleanup {
    child: Option<Child>,
    child_exited_unreaped: bool,
    containment: ChildContainment,
    stdout_reader: Option<PipeReader>,
    stderr_reader: Option<PipeReader>,
    completion: Option<mpsc::Sender<()>>,
}

impl PendingChildCleanup {
    fn poll(&mut self) -> bool {
        if let Some(child) = self.child.as_mut() {
            self.containment.terminate(Some(child));
            if !self.child_exited_unreaped {
                let observation = observe_child_exit(child);
                if observation.status.is_some() {
                    self.child = None;
                } else {
                    self.child_exited_unreaped = observation.exited_unreaped;
                }
            }
        }

        Self::join_finished_reader(&mut self.stdout_reader);
        Self::join_finished_reader(&mut self.stderr_reader);
        if self.child_exited_unreaped
            && self.stdout_reader.is_none()
            && self.stderr_reader.is_none()
            && let Some(child) = self.child.as_mut()
        {
            self.containment.terminate(None);
            if child.wait().is_ok() {
                self.child = None;
                self.child_exited_unreaped = false;
            }
        }
        #[cfg(windows)]
        if self.child.is_none() && (self.stdout_reader.is_some() || self.stderr_reader.is_some()) {
            self.containment.terminate(None);
        }

        let complete =
            self.child.is_none() && self.stdout_reader.is_none() && self.stderr_reader.is_none();
        if complete && let Some(completion) = self.completion.take() {
            let _notified = completion.send(());
        }
        complete
    }

    fn join_finished_reader(reader: &mut Option<PipeReader>) {
        if reader.as_ref().is_some_and(|reader| reader.is_finished())
            && let Some(reader) = reader.take()
        {
            let _result = reader.join();
        }
    }

    fn finish(mut self) {
        while !self.poll() {
            thread::sleep(CHILD_PROCESS_POLL);
        }
    }
}

struct ChildReaper {
    sender: mpsc::Sender<PendingChildCleanup>,
    _worker: JoinHandle<()>,
}

impl ChildReaper {
    fn enqueue(&self, cleanup: PendingChildCleanup) -> Option<PendingChildCleanup> {
        self.sender.send(cleanup).err().map(|error| error.0)
    }
}

fn enqueue_child_cleanup(reaper: &ChildReaper, cleanup: PendingChildCleanup) {
    if let Some(cleanup) = reaper.enqueue(cleanup) {
        cleanup.finish();
    }
}

static CHILD_REAPER: LazyLock<Option<ChildReaper>> = LazyLock::new(|| {
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("mado-pilot-child-reaper".to_owned())
        .spawn(move || child_reaper_loop(receiver))
        .ok()?;
    Some(ChildReaper {
        sender,
        _worker: worker,
    })
});

fn child_reaper() -> Option<&'static ChildReaper> {
    CHILD_REAPER.as_ref()
}

fn child_reaper_loop(receiver: mpsc::Receiver<PendingChildCleanup>) {
    let mut pending = Vec::new();
    let mut disconnected = false;
    loop {
        if pending.is_empty() {
            if disconnected {
                break;
            }
            match receiver.recv() {
                Ok(cleanup) => pending.push(cleanup),
                Err(_) => break,
            }
        } else if disconnected {
            thread::sleep(CHILD_PROCESS_POLL);
        } else {
            match receiver.recv_timeout(CHILD_PROCESS_POLL) {
                Ok(cleanup) => pending.push(cleanup),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            }
        }
        pending.extend(receiver.try_iter());

        let mut index = 0;
        while index < pending.len() {
            if pending[index].poll() {
                drop(pending.swap_remove(index));
            } else {
                index += 1;
            }
        }
    }
}

trait PrimaryChildCleanup {
    fn terminate_and_wait(
        &mut self,
        containment: &mut ChildContainment,
        child: &mut Child,
        deadline: Instant,
    ) -> ChildExitObservation {
        containment.terminate(Some(child));
        wait_for_child_exit(child, deadline)
    }

    fn reaper_completion(&mut self) -> Option<mpsc::Sender<()>> {
        None
    }

    fn reader_events(&mut self) -> Option<mpsc::Sender<PipeReaderEvent>> {
        None
    }
}

struct DefaultPrimaryChildCleanup;

impl PrimaryChildCleanup for DefaultPrimaryChildCleanup {}

fn failed_child_output() -> BoundedChildOutput {
    BoundedChildOutput {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        within_bounds: false,
    }
}

fn handoff_reader_spawn_failure<C: PrimaryChildCleanup>(
    reaper: &ChildReaper,
    mut child: Child,
    mut containment: ChildContainment,
    stdout_reader: Option<PipeReader>,
    stderr_reader: Option<PipeReader>,
    primary_cleanup: &mut C,
) {
    containment.terminate(Some(&mut child));
    enqueue_child_cleanup(
        reaper,
        PendingChildCleanup {
            child: Some(child),
            child_exited_unreaped: false,
            containment,
            stdout_reader,
            stderr_reader,
            completion: primary_cleanup.reaper_completion(),
        },
    );
}

/// Runs one benchmark child with finite time and per-stream output bounds.
///
/// A child still running at `wait` is terminated as one contained process tree
/// and given one bounded second to be reaped. Output beyond `max_output_bytes`
/// is drained so the child cannot deadlock on a full pipe, but it is not
/// retained and makes [`BoundedChildOutput::within_bounds`] false. Cleanup that
/// outlives either bounded allowance is transferred intact to one dedicated
/// reaper, which retains every live child and reader until process exit and pipe
/// EOF.
pub fn bounded_child_output(
    command: &mut Command,
    wait: Duration,
    max_output_bytes: usize,
) -> BoundedChildOutput {
    bounded_child_output_checked(command, wait, max_output_bytes, |_| true)
}

/// Runs one bounded child and rejects it unless `check` accepts its live PID.
pub fn bounded_child_output_checked(
    command: &mut Command,
    wait: Duration,
    max_output_bytes: usize,
    check: impl FnOnce(u32) -> bool,
) -> BoundedChildOutput {
    let mut primary_cleanup = DefaultPrimaryChildCleanup;
    bounded_child_output_with(command, wait, max_output_bytes, &mut primary_cleanup, check)
}

fn bounded_child_output_with<C: PrimaryChildCleanup>(
    command: &mut Command,
    wait: Duration,
    max_output_bytes: usize,
    primary_cleanup: &mut C,
    check: impl FnOnce(u32) -> bool,
) -> BoundedChildOutput {
    let Some(reaper) = child_reaper() else {
        return failed_child_output();
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let Some(prepared_containment) = PreparedChildContainment::prepare(command) else {
        return failed_child_output();
    };
    let Ok(mut child) = command.spawn() else {
        return failed_child_output();
    };
    let (mut containment, contained) = prepared_containment.attach(&child);
    let spawn_accepted =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(child.id())))
            .unwrap_or(false);
    let stdout = child
        .stdout
        .take()
        .expect("a piped benchmark child must expose stdout");
    let stderr = child
        .stderr
        .take()
        .expect("a piped benchmark child must expose stderr");
    let reader_events = primary_cleanup.reader_events();
    let stdout_reader = match spawn_capped_pipe_reader(
        stdout,
        max_output_bytes,
        PipeStream::Stdout,
        reader_events.clone(),
    ) {
        Ok(reader) => reader,
        Err(_) => {
            drop(stderr);
            handoff_reader_spawn_failure(reaper, child, containment, None, None, primary_cleanup);
            return failed_child_output();
        }
    };
    let stderr_reader =
        match spawn_capped_pipe_reader(stderr, max_output_bytes, PipeStream::Stderr, reader_events)
        {
            Ok(reader) => reader,
            Err(_) => {
                handoff_reader_spawn_failure(
                    reaper,
                    child,
                    containment,
                    Some(stdout_reader),
                    None,
                    primary_cleanup,
                );
                return failed_child_output();
            }
        };

    let activated = contained && spawn_accepted && containment.activate(&child);
    let deadline = Instant::now() + wait;
    let mut exit = if activated {
        wait_for_child_exit(&mut child, deadline)
    } else {
        ChildExitObservation::default()
    };
    let exited_in_time = activated && exit.exited();
    if !exited_in_time {
        exit = primary_cleanup.terminate_and_wait(
            &mut containment,
            &mut child,
            Instant::now() + CHILD_PROCESS_TERMINATE_WAIT,
        );
    }

    let drain_deadline = Instant::now() + CHILD_PIPE_DRAIN_WAIT;
    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
        && Instant::now() < drain_deadline
    {
        thread::sleep(
            drain_deadline
                .saturating_duration_since(Instant::now())
                .min(CHILD_PROCESS_POLL),
        );
    }
    let (stdout, stdout_reader) = if stdout_reader.is_finished() {
        (stdout_reader.join().ok(), None)
    } else {
        (None, Some(stdout_reader))
    };
    let (stderr, stderr_reader) = if stderr_reader.is_finished() {
        (stderr_reader.join().ok(), None)
    } else {
        (None, Some(stderr_reader))
    };

    if exit.exited() {
        containment.terminate(None);
    } else {
        containment.terminate(Some(&mut child));
    }
    let readers_pending = stdout_reader.is_some() || stderr_reader.is_some();
    let mut status = exit.status;
    let mut child_exited_unreaped = exit.exited_unreaped;
    if child_exited_unreaped && !readers_pending {
        status = child.wait().ok();
        child_exited_unreaped = status.is_none();
    }

    let needs_reaper = status.is_none() || readers_pending;
    if needs_reaper {
        enqueue_child_cleanup(
            reaper,
            PendingChildCleanup {
                child: if status.is_none() { Some(child) } else { None },
                child_exited_unreaped,
                containment,
                stdout_reader,
                stderr_reader,
                completion: primary_cleanup.reaper_completion(),
            },
        );
    }

    let within_bounds = exited_in_time
        && status.is_some()
        && stdout
            .as_ref()
            .is_some_and(|pipe| pipe.complete && !pipe.overflowed)
        && stderr
            .as_ref()
            .is_some_and(|pipe| pipe.complete && !pipe.overflowed);
    BoundedChildOutput {
        status,
        stdout: stdout.map_or_else(Vec::new, |pipe| pipe.bytes),
        stderr: stderr.map_or_else(Vec::new, |pipe| pipe.bytes),
        within_bounds,
    }
}

/// The target triple the benchmark runs on, when it is one this project
/// releases.
///
/// Selected rather than detected. `std::env::consts` can report the
/// architecture and the operating system but not the vendor or the ABI, and a
/// triple assembled from the parts that are available would be a guess printed
/// where a measurement condition belongs. A budget is valid only for the target
/// in its profile, so the wrong string here is worse than no string.
pub const RELEASE_TARGET: &str = if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
    "aarch64-apple-darwin"
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc"
)) {
    "x86_64-pc-windows-msvc"
} else {
    "not a declared release target"
};

// --- Allocation accounting ---------------------------------------------------

/// Live heap bytes, and the high-water mark since it was last reset.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The system allocator, counting what it hands out and takes back.
///
/// Resident memory is what `docs/performance.md` names for `peak_memory` and
/// `steady_memory`, and it is the wrong instrument here. It is measured through
/// a different platform API on each release target, it moves with allocator and
/// operating-system behaviour that no MadoPilot change can affect, and on a
/// workload this small the noise is larger than the signal. Live heap bytes are
/// portable, are the same computation on both targets, and answer the question
/// a bounded-memory gate actually asks: does a repeated operation give back
/// what it took. The three measures this feeds are named separately in the
/// measure vocabulary so that neither reading is mistaken for the other.
///
/// A benchmark installs it with:
///
/// ```ignore
/// #[global_allocator]
/// static ALLOCATOR: mado_pilot_testkit::bench_harness::Accounting =
///     mado_pilot_testkit::bench_harness::Accounting;
/// ```
#[derive(Debug)]
pub struct Accounting;

// SAFETY: every method forwards to the system allocator with the layout it was
// given and returns exactly what it returned. The counters are plain relaxed
// arithmetic on the side and never influence which pointer is produced.
unsafe impl GlobalAlloc for Accounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is passed through unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(0, layout.size());
        // SAFETY: as above.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            record(new_size, layout.size());
        }
        moved
    }
}

/// Applies one allocation and one release to the counters.
fn record(gained: usize, lost: usize) {
    let before = LIVE.fetch_add(gained, Ordering::Relaxed) + gained;
    LIVE.fetch_sub(lost, Ordering::Relaxed);

    // A peak that another thread raised higher stays; this only ever lifts it.
    PEAK.fetch_max(before, Ordering::Relaxed);
}

/// Live heap bytes now.
fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

// --- Running -----------------------------------------------------------------

/// How many iterations a run discards and how many it keeps.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    warmup: usize,
    samples: usize,
}

impl Plan {
    /// Builds an explicit plan for a workload set whose contract uses a
    /// different sample schedule from the Phase 1 default.
    ///
    /// # Panics
    ///
    /// Panics when `samples` is zero, because a run with no retained sample
    /// cannot produce a percentile or exercise a correctness oracle.
    #[must_use]
    pub fn new(warmup: usize, samples: usize) -> Self {
        assert!(samples > 0, "a benchmark plan retains at least one sample");
        Self { warmup, samples }
    }

    /// Enough samples for the oracles, not enough for a percentile.
    #[must_use]
    pub const fn smoke() -> Self {
        Self {
            warmup: 1,
            samples: 3,
        }
    }

    /// A full timing run.
    #[must_use]
    pub const fn full() -> Self {
        Self {
            warmup: 20,
            samples: 200,
        }
    }

    /// Returns the plan a run's arguments ask for.
    ///
    /// `cargo bench` passes `--bench`; `cargo test --all-targets` does not, and
    /// wants the oracles rather than the timings.
    #[must_use]
    pub fn from(arguments: &[String]) -> Self {
        if arguments.iter().any(|argument| argument == "--bench") {
            Self::full()
        } else {
            Self::smoke()
        }
    }

    /// How many samples one workload retains.
    #[must_use]
    pub const fn samples(self) -> usize {
        self.samples
    }

    /// How many iterations a run discards before retaining samples.
    #[must_use]
    pub const fn warmup(self) -> usize {
        self.warmup
    }
}

/// Native GPU-resource costs observed while producing one benchmark result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureResources {
    /// Bytes copied out of producer-owned surfaces.
    pub copied_bytes: u64,
    /// Maximum simultaneously live Adapter-owned detached textures.
    pub detached_textures_peak: u64,
    /// Maximum simultaneously live CPU-readable staging textures.
    pub staging_textures_peak: u64,
    /// Maximum simultaneously live producer, detached, and staging textures.
    pub gpu_resources_peak: u64,
}

/// What one iteration of a workload reports.
#[derive(Debug)]
pub struct Sample {
    elapsed: Duration,
    correct: bool,
    mapped: u64,
    peak_resident: Option<u64>,
    stale: Option<(u64, u64)>,
    capture_resources: Option<CaptureResources>,
}

impl Sample {
    /// A sample from a workload that maps frame bytes.
    #[must_use]
    pub const fn new(elapsed: Duration, correct: bool, mapped: u64) -> Self {
        Self {
            elapsed,
            correct,
            mapped,
            peak_resident: None,
            stale: None,
            capture_resources: None,
        }
    }

    /// A sample from a workload that maps nothing.
    #[must_use]
    pub const fn unmapped(elapsed: Duration, correct: bool) -> Self {
        Self::new(elapsed, correct, 0)
    }

    /// Associates an observable stale/drop count with this sample.
    ///
    /// `total` is the number of producer publications represented by the
    /// sample, including the one returned to the consumer. The ratio is
    /// therefore `stale / total`, never a count detached from its denominator.
    #[must_use]
    pub const fn with_stale_work(mut self, stale: u64, total: u64) -> Self {
        self.stale = Some((stale, total));
        self
    }

    /// Associates the measured native process's peak resident set with this sample.
    ///
    /// This is separate from the Rust global-allocator counters because it
    /// includes native allocations and may describe either the benchmark
    /// process itself or a separately linked C or C++ child after owned-handle
    /// cleanup.
    #[must_use]
    pub const fn with_peak_resident_bytes(mut self, bytes: u64) -> Self {
        self.peak_resident = Some(bytes);
        self
    }

    /// Associates native capture-copy and GPU-resource costs with this sample.
    #[must_use]
    pub const fn with_capture_resources(mut self, resources: CaptureResources) -> Self {
        self.capture_resources = Some(resources);
        self
    }
}

/// One workload's samples, and what they cost besides time.
#[derive(Debug)]
pub struct Workload {
    name: &'static str,
    oracle: &'static str,
    warmup_iterations: usize,
    sample_count: usize,
    elapsed: Vec<Duration>,
    incorrect: usize,
    stale: u64,
    scheduled: u64,
    mapped: u64,
    iteration_span: Duration,
    peak_bytes: usize,
    steady_bytes: usize,
    peak_resident_bytes: Option<u64>,
    copied_bytes: Option<u64>,
    detached_textures_peak: Option<u64>,
    staging_textures_peak: Option<u64>,
    gpu_resources_peak: Option<u64>,
    growth_bytes: i64,
}

impl Workload {
    /// How many retained samples failed their oracle.
    #[must_use]
    pub const fn incorrect(&self) -> usize {
        self.incorrect
    }

    /// Returns the `percentile`-th sample, in milliseconds.
    #[must_use]
    pub fn percentile(&self, percentile: f64) -> f64 {
        let mut sorted = self.elapsed.clone();
        sorted.sort_unstable();
        let Some(last) = sorted.len().checked_sub(1) else {
            return 0.0;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the clamped nearest rank indexes this small in-memory sample vector"
        )]
        let index = ((percentile.clamp(0.0, 1.0) * sorted.len() as f64)
            .ceil()
            .max(1.0) as usize
            - 1)
        .min(last);
        sorted[index].as_secs_f64() * 1_000.0
    }
    /// Returns the slowest retained sample.
    ///
    /// This is the hard scenario-bound observation. Percentiles can hide one
    /// path that exceeded its absolute deadline, so qualification profiles use
    /// both.
    #[must_use]
    pub fn max_elapsed(&self) -> Duration {
        self.elapsed.iter().copied().max().unwrap_or_default()
    }

    /// Returns one sampled iteration's cost in milliseconds, from a single span.
    ///
    /// A per-iteration percentile disappears when the operation is faster than
    /// the host clock can express — on `x86_64-pc-windows-msvc` a
    /// matching-format frame mapping measures exactly zero, because it is a
    /// reference-count increment. One clock read across hundreds of iterations
    /// recovers a number that granularity cannot swallow.
    ///
    /// It measures more than the operation does. Everything an iteration needs
    /// is inside the span: preparing its inputs, checking the oracle, dropping
    /// what it produced. So it is an upper bound on the operation rather than a
    /// reading of it, and that is what makes it usable as a ceiling for a
    /// workload whose own fast path is too quick to time.
    #[must_use]
    pub fn iteration_span_ms(&self) -> f64 {
        self.iteration_span.as_secs_f64() * 1_000.0
    }

    /// Live heap bytes this workload's samples did not give back.
    ///
    /// Signed, because a workload that ends below its post-warmup baseline has
    /// released more than it took and satisfies the requirement just as a
    /// workload that ended level does.
    #[must_use]
    pub const fn growth_bytes(&self) -> i64 {
        self.growth_bytes
    }
    /// High-water live Rust heap bytes attributable to this workload.
    #[must_use]
    pub const fn peak_allocated_bytes(&self) -> usize {
        self.peak_bytes
    }

    /// Maximum bytes mapped by one retained result.
    #[must_use]
    pub const fn mapped_bytes_per_result(&self) -> u64 {
        self.mapped
    }

    /// Peak resident bytes reported for this native workload.
    #[must_use]
    pub const fn peak_resident_bytes(&self) -> Option<u64> {
        self.peak_resident_bytes
    }

    /// Maximum producer-surface bytes copied during one retained sample.
    #[must_use]
    pub const fn copied_bytes(&self) -> Option<u64> {
        self.copied_bytes
    }

    /// Maximum simultaneously live detached textures.
    #[must_use]
    pub const fn detached_textures_peak(&self) -> Option<u64> {
        self.detached_textures_peak
    }

    /// Maximum simultaneously live CPU-readable staging textures.
    #[must_use]
    pub const fn staging_textures_peak(&self) -> Option<u64> {
        self.staging_textures_peak
    }

    /// Maximum simultaneously live producer, detached, and staging textures.
    #[must_use]
    pub const fn gpu_resources_peak(&self) -> Option<u64> {
        self.gpu_resources_peak
    }

    /// Share of observed producer work skipped before a retained result.
    #[must_use]
    pub fn stale_work_ratio(&self) -> Option<f64> {
        (self.scheduled > 0).then(|| self.stale as f64 / self.scheduled as f64)
    }

    /// The workload's name, as the report files it under.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

struct WorkloadAccumulator {
    name: &'static str,
    oracle: &'static str,
    elapsed: Vec<Duration>,
    incorrect: usize,
    mapped: u64,
    peak_resident_bytes: Option<u64>,
    copied_bytes: Option<u64>,
    detached_textures_peak: Option<u64>,
    staging_textures_peak: Option<u64>,
    gpu_resources_peak: Option<u64>,
    stale: u64,
    scheduled: u64,
}

impl WorkloadAccumulator {
    fn new(name: &'static str, oracle: &'static str, samples: usize) -> Self {
        Self {
            name,
            oracle,
            elapsed: Vec::with_capacity(samples),
            incorrect: 0,
            mapped: 0,
            peak_resident_bytes: None,
            copied_bytes: None,
            detached_textures_peak: None,
            staging_textures_peak: None,
            gpu_resources_peak: None,
            stale: 0,
            scheduled: 0,
        }
    }

    fn observe(&mut self, sample: Sample) {
        if !sample.correct {
            self.incorrect += 1;
        }
        if let Some((sample_stale, sample_total)) = sample.stale {
            self.stale = self.stale.saturating_add(sample_stale);
            self.scheduled = self.scheduled.saturating_add(sample_total);
        }
        self.mapped = self.mapped.max(sample.mapped);
        if let Some(sample_peak) = sample.peak_resident {
            self.peak_resident_bytes = Some(
                self.peak_resident_bytes
                    .unwrap_or_default()
                    .max(sample_peak),
            );
        }
        if let Some(resources) = sample.capture_resources {
            self.copied_bytes = Some(
                self.copied_bytes
                    .unwrap_or_default()
                    .max(resources.copied_bytes),
            );
            self.detached_textures_peak = Some(
                self.detached_textures_peak
                    .unwrap_or_default()
                    .max(resources.detached_textures_peak),
            );
            self.staging_textures_peak = Some(
                self.staging_textures_peak
                    .unwrap_or_default()
                    .max(resources.staging_textures_peak),
            );
            self.gpu_resources_peak = Some(
                self.gpu_resources_peak
                    .unwrap_or_default()
                    .max(resources.gpu_resources_peak),
            );
        }
        self.elapsed.push(sample.elapsed);
    }

    fn finish(
        self,
        plan: Plan,
        iteration_span: Duration,
        before_fixture: usize,
        after_warmup: usize,
        ending: usize,
    ) -> Workload {
        Workload {
            name: self.name,
            oracle: self.oracle,
            warmup_iterations: plan.warmup,
            sample_count: plan.samples,
            elapsed: self.elapsed,
            incorrect: self.incorrect,
            mapped: self.mapped,
            iteration_span,
            peak_bytes: PEAK.load(Ordering::Relaxed).saturating_sub(before_fixture),
            steady_bytes: ending.saturating_sub(before_fixture),
            peak_resident_bytes: self.peak_resident_bytes,
            copied_bytes: self.copied_bytes,
            detached_textures_peak: self.detached_textures_peak,
            staging_textures_peak: self.staging_textures_peak,
            gpu_resources_peak: self.gpu_resources_peak,
            stale: self.stale,
            scheduled: self.scheduled,
            growth_bytes: i64::try_from(ending).unwrap_or(i64::MAX)
                - i64::try_from(after_warmup).unwrap_or(i64::MAX),
        }
    }
}

/// Runs `workload` through its warmup and retained samples.
///
/// Progress records go to stderr before and after the measured region. They
/// keep long native runs observable without perturbing individual sample
/// latency or the post-warmup allocation counters.
///
/// The three memory numbers are differences against two baselines rather than
/// absolute totals, because an absolute total would include every earlier
/// workload's retained samples and would grow down the report for a reason that
/// has nothing to do with the workload being measured. The fixture baseline is
/// what this workload's own footprint is measured against; the post-warmup one
/// is what its growth is measured against, so a one-time cost the first
/// iterations paid is not reported as a leak.
pub fn measure<F, M>(
    name: &'static str,
    oracle: &'static str,
    plan: Plan,
    make: M,
    workload: fn(&F) -> Sample,
) -> Workload
where
    M: FnOnce() -> F,
{
    eprintln!(
        "benchmark-progress workload={name} phase=setup warmups={} samples={}",
        plan.warmup, plan.samples
    );
    let mut result = WorkloadAccumulator::new(name, oracle, plan.samples);
    let before_fixture = live();
    let fixture = make();

    eprintln!("benchmark-progress workload={name} phase=warmup");
    for _ in 0..plan.warmup {
        workload(&fixture);
    }
    eprintln!("benchmark-progress workload={name} phase=sampling");
    let after_warmup = live();
    PEAK.store(after_warmup, Ordering::Relaxed);

    let span = Instant::now();
    for _ in 0..plan.samples {
        result.observe(workload(&fixture));
    }
    let span = span.elapsed();
    let ending = live();
    eprintln!(
        "benchmark-progress workload={name} phase=complete elapsed_ms={:.3}",
        span.as_secs_f64() * 1_000.0
    );

    result.finish(
        plan,
        span / u32::try_from(plan.samples).unwrap_or(u32::MAX),
        before_fixture,
        after_warmup,
        ending,
    )
}

/// Measures two latency views of the same native operation.
///
/// This is for a workload whose one expensive system interaction produces two
/// independent timing results. It invokes the operation once per warmup and
/// retained iteration, avoiding duplicate capture, mapping, and GPU memory.
pub fn measure_pair<F, M>(
    first: (&'static str, &'static str),
    second: (&'static str, &'static str),
    plan: Plan,
    make: M,
    workload: fn(&F) -> (Sample, Sample),
) -> [Workload; 2]
where
    M: FnOnce() -> F,
{
    eprintln!(
        "benchmark-progress workloads={},{} phase=setup warmups={} samples={}",
        first.0, second.0, plan.warmup, plan.samples
    );
    let mut first_result = WorkloadAccumulator::new(first.0, first.1, plan.samples);
    let mut second_result = WorkloadAccumulator::new(second.0, second.1, plan.samples);
    let before_fixture = live();
    let fixture = make();

    eprintln!(
        "benchmark-progress workloads={},{} phase=warmup",
        first.0, second.0
    );
    for _ in 0..plan.warmup {
        workload(&fixture);
    }
    eprintln!(
        "benchmark-progress workloads={},{} phase=sampling",
        first.0, second.0
    );
    let after_warmup = live();
    PEAK.store(after_warmup, Ordering::Relaxed);

    let span = Instant::now();
    for _ in 0..plan.samples {
        let (first_sample, second_sample) = workload(&fixture);
        first_result.observe(first_sample);
        second_result.observe(second_sample);
    }
    let span = span.elapsed();
    let ending = live();
    eprintln!(
        "benchmark-progress workloads={},{} phase=complete elapsed_ms={:.3}",
        first.0,
        second.0,
        span.as_secs_f64() * 1_000.0
    );

    let iteration_span = span / u32::try_from(plan.samples).unwrap_or(u32::MAX);
    [
        first_result.finish(plan, iteration_span, before_fixture, after_warmup, ending),
        second_result.finish(plan, iteration_span, before_fixture, after_warmup, ending),
    ]
}

// --- Reporting ---------------------------------------------------------------

/// What the run is, for the report's `[benchmark]` table.
#[derive(Debug)]
pub struct Benchmark {
    /// The identifier a committed profile is filed under.
    pub id: &'static str,
    /// One sentence naming what the set of workloads covers.
    pub workload: &'static str,
    /// The phase that introduced them.
    pub phase: &'static str,
}

/// The conditions that make a measurement reproducible.
///
/// `hardware` and `os_version` are the operator's to state and are read from
/// the command line: a CPU model the program detected would be a guess recorded
/// as a measurement condition.
#[derive(Debug)]
pub struct Profile {
    /// Tracked paths of everything the workloads read.
    pub fixture: String,
    /// One digest pinning all of it.
    pub fixture_sha256: String,
    /// Digest of the executable that performed the measurement, when retained.
    pub benchmark_executable_sha256: Option<String>,
    /// The machine, as the operator stated it.
    pub hardware: String,
    /// Its operating-system version, as the operator stated it.
    pub os_version: String,
    /// The minimum operating-system version the measured artifacts target.
    pub deployment_target: Option<String>,
    /// The command profile and feature selection that produced the executable.
    pub build_profile: String,
    /// How every retained sample was checked.
    pub correctness_oracle: &'static str,
    /// The queue depth and drop policy in effect.
    pub queue_policy: &'static str,
    /// Optional target-specific conditions not represented by another field.
    pub notes: Option<String>,
}

impl Profile {
    /// Reads `--hardware` and `--os-version`, falling back to `--label`.
    #[must_use]
    pub fn host(arguments: &[String]) -> (String, String) {
        // `--label` predates the two specific arguments and named the host as
        // one string. It still fills the hardware field so an older recorded
        // command keeps working.
        let label = argument(arguments, "--label");
        (
            argument(arguments, "--hardware")
                .or(label)
                .unwrap_or_else(|| "unstated".to_owned()),
            argument(arguments, "--os-version").unwrap_or_else(|| "unstated".to_owned()),
        )
    }
}

/// The `[benchmark]` block a report opens with, as `key = value` lines.
///
/// Returned as a list rather than printed one line at a time so that the key
/// set is a value something can compare. A committed profile is this block with
/// `status` and `normative` answered differently and the budgets added, so a key
/// here that no profile carries — or one every profile carries and this omits —
/// is the two records drifting apart. `benchmark_block_drift.rs` is that
/// comparison.
#[must_use]
pub fn benchmark_block(benchmark: &Benchmark) -> Vec<(&'static str, String)> {
    vec![
        ("id", format!("\"{}\"", escape(benchmark.id))),
        ("workload", format!("\"{}\"", escape(benchmark.workload))),
        ("phase", format!("\"{}\"", escape(benchmark.phase))),
        // What this run is: harness output, which nothing gates on, carrying
        // measurements that are real readings rather than illustrations.
        ("status", "\"harness-output\"".to_owned()),
        ("normative", "false".to_owned()),
        ("measurements_recorded", "true".to_owned()),
    ]
}

/// Prints a profile-shaped report with no budget in it.
///
/// A committed file under `docs/benchmarks/` is this output with budgets added.
pub fn report(benchmark: &Benchmark, profile: &Profile, plan: Plan, workloads: &[Workload]) {
    println!("format_version = 1");
    println!();
    println!("[benchmark]");
    for (key, value) in benchmark_block(benchmark) {
        println!("{key} = {value}");
    }
    println!("# A committed profile under docs/benchmarks/ carries the budgets.");
    println!();
    println!("[profile]");
    println!("fixture = \"{}\"", escape(&profile.fixture));
    println!("fixture_sha256 = \"{}\"", escape(&profile.fixture_sha256));
    if let Some(digest) = &profile.benchmark_executable_sha256 {
        println!("benchmark_executable_sha256 = \"{}\"", escape(digest));
    }
    println!("release_target = \"{RELEASE_TARGET}\"");
    println!("hardware = \"{}\"", escape(&profile.hardware));
    println!("os_version = \"{}\"", escape(&profile.os_version));
    if let Some(target) = &profile.deployment_target {
        println!("deployment_target = \"{}\"", escape(target));
    }
    println!("build_profile = \"{}\"", escape(&profile.build_profile));
    println!("warmup_iterations = {}", plan.warmup);
    println!("sample_count = {}", plan.samples);
    println!("correctness_oracle = \"{}\"", profile.correctness_oracle);
    println!("queue_policy = \"{}\"", profile.queue_policy);
    if let Some(notes) = &profile.notes {
        println!("notes = \"{}\"", escape(notes));
    }
    println!();

    for workload in workloads {
        println!("[[measurement]]");
        println!("workload = \"{}\"", workload.name);
        println!("correctness_oracle = \"{}\"", workload.oracle);
        if workload.warmup_iterations != plan.warmup {
            println!("warmup_iterations = {}", workload.warmup_iterations);
        }
        if workload.sample_count != plan.samples {
            println!("sample_count = {}", workload.sample_count);
        }
        println!("result_correctness = {}", workload.incorrect);
        println!("latency_p50_ms = {:.6}", workload.percentile(0.50));
        println!("latency_p95_ms = {:.6}", workload.percentile(0.95));
        println!(
            "latency_max_ms = {:.6}",
            workload.max_elapsed().as_secs_f64() * 1_000.0
        );
        println!("iteration_span_ms = {:.6}", workload.iteration_span_ms());
        println!("mapped_bytes_per_result = {}", workload.mapped);
        if let Some(bytes) = workload.copied_bytes {
            println!("copied_bytes_per_result = {bytes}");
        }
        if let Some(textures) = workload.detached_textures_peak {
            println!("detached_textures_peak = {textures}");
        }
        if let Some(textures) = workload.staging_textures_peak {
            println!("staging_textures_peak = {textures}");
        }
        if let Some(resources) = workload.gpu_resources_peak {
            println!("gpu_resources_peak = {resources}");
        }
        if let Some(ratio) = workload.stale_work_ratio() {
            println!("stale_work_ratio = {ratio:.9}");
        }
        println!("peak_allocated_bytes = {}", workload.peak_bytes);
        println!("steady_allocated_bytes = {}", workload.steady_bytes);
        println!("allocated_growth_bytes = {}", workload.growth_bytes);
        if let Some(bytes) = workload.peak_resident_bytes {
            println!("peak_resident_bytes = {bytes}");
        }
        println!();
    }
}

/// Prints the short line a `cargo test` run reports instead of a profile.
pub fn summarize(name: &str, plan: Plan, workloads: &[Workload]) {
    let failures: usize = workloads.iter().map(Workload::incorrect).sum();
    println!(
        "{name}: {} workloads, {} samples each, {failures} oracle failure(s)",
        workloads.len(),
        plan.samples
    );
}

/// One frozen latency gate for a named workload.
///
/// These values are fixed before a native qualification run. They are kept out
/// of [`measure`] because most profiles establish host-specific ceilings only
/// after measurement; callers opt in only when a pre-measurement plan already
/// fixed all three bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyBudget {
    workload: &'static str,
    p50: Duration,
    p95: Duration,
    hard_max: Duration,
}

impl LatencyBudget {
    /// Builds one pre-measurement latency gate.
    #[must_use]
    pub const fn new(
        workload: &'static str,
        p50: Duration,
        p95: Duration,
        hard_max: Duration,
    ) -> Self {
        Self {
            workload,
            p50,
            p95,
            hard_max,
        }
    }

    /// Returns the workload name this gate applies to.
    #[must_use]
    pub const fn workload(self) -> &'static str {
        self.workload
    }

    /// Returns the frozen p50 ceiling.
    #[must_use]
    pub const fn p50(self) -> Duration {
        self.p50
    }

    /// Returns the frozen p95 ceiling.
    #[must_use]
    pub const fn p95(self) -> Duration {
        self.p95
    }

    /// Returns the frozen per-scenario maximum.
    #[must_use]
    pub const fn hard_max(self) -> Duration {
        self.hard_max
    }
}

/// Phase 2.2 controlled-capture latency ceilings frozen before qualification.
pub const PHASE2_2_CAPTURE_LATENCY_BUDGETS: [LatencyBudget; 2] = [
    LatencyBudget::new(
        "fixture_command_acknowledgement",
        Duration::from_millis(50),
        Duration::from_millis(100),
        Duration::from_millis(500),
    ),
    LatencyBudget::new(
        "controlled_stimulus_to_frame",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
];

/// Phase 2.2 controlled-transition latency ceilings frozen before qualification.
pub const PHASE2_2_TRANSITION_LATENCY_BUDGETS: [LatencyBudget; 1] = [LatencyBudget::new(
    "close_drain",
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_secs(1),
)];

/// Phase 2 macOS production-capture latency ceilings accepted by ADR 0030.
pub const PHASE2_PRODUCTION_CAPTURE_LATENCY_BUDGETS: [LatencyBudget; 5] = [
    LatencyBudget::new(
        "publication_age",
        Duration::from_millis(5),
        Duration::from_millis(15),
        Duration::from_millis(50),
    ),
    LatencyBudget::new(
        "steady_frame_acquisition",
        Duration::from_millis(75),
        Duration::from_millis(150),
        Duration::from_millis(250),
    ),
    LatencyBudget::new(
        "latest_acquisition",
        Duration::from_millis(1),
        Duration::from_millis(1),
        Duration::from_millis(1),
    ),
    LatencyBudget::new(
        "cpu_map_bgra8",
        Duration::from_millis(1),
        Duration::from_millis(2),
        Duration::from_millis(10),
    ),
    LatencyBudget::new(
        "retained_pressure_resume",
        Duration::from_millis(10),
        Duration::from_millis(50),
        Duration::from_millis(75),
    ),
];

/// Phase 2 macOS production-transition latency ceilings accepted by ADR 0030.
pub const PHASE2_PRODUCTION_TRANSITION_LATENCY_BUDGETS: [LatencyBudget; 3] = [
    LatencyBudget::new(
        "open_first_frame",
        Duration::from_millis(350),
        Duration::from_millis(350),
        Duration::from_millis(400),
    ),
    LatencyBudget::new(
        "resize_recreation",
        Duration::from_millis(175),
        Duration::from_millis(250),
        Duration::from_millis(300),
    ),
    LatencyBudget::new(
        "close_drain",
        Duration::from_millis(250),
        Duration::from_millis(300),
        Duration::from_millis(300),
    ),
];

/// Maximum live Rust heap for the macOS production-capture profile.
pub const PHASE2_PRODUCTION_CAPTURE_HEAP_LIMIT_BYTES: usize = 32 * 1_024 * 1_024;

/// Maximum live Rust heap for the macOS production-transition profile.
pub const PHASE2_PRODUCTION_TRANSITION_HEAP_LIMIT_BYTES: usize = 16 * 1_024 * 1_024;

/// Maximum mapped bytes for one macOS production fixture frame.
pub const PHASE2_PRODUCTION_MAPPED_BYTES_LIMIT: u64 = 4_628_480;

/// Phase 2 Windows 1280x720 production-capture ceilings accepted by ADR 0031.
pub const PHASE2_WINDOWS_PRODUCTION_1280_LATENCY_BUDGETS: [LatencyBudget; 4] = [
    LatencyBudget::new(
        "steady_frame_acquisition",
        Duration::from_millis(75),
        Duration::from_millis(150),
        Duration::from_millis(200),
    ),
    LatencyBudget::new(
        "callback_copy",
        Duration::from_micros(500),
        Duration::from_micros(1_500),
        Duration::from_millis(5),
    ),
    LatencyBudget::new(
        "latest_acquisition",
        Duration::from_micros(5),
        Duration::from_micros(15),
        Duration::from_micros(150),
    ),
    LatencyBudget::new(
        "cpu_map_bgra8",
        Duration::from_millis(6),
        Duration::from_millis(15),
        Duration::from_millis(20),
    ),
];

/// Phase 2 Windows 1280x720 production-transition ceilings accepted by ADR 0031.
pub const PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_LATENCY_BUDGETS: [LatencyBudget; 5] = [
    LatencyBudget::new(
        "open_first_frame",
        Duration::from_millis(350),
        Duration::from_millis(350),
        Duration::from_millis(350),
    ),
    LatencyBudget::new(
        "retained_pressure_resume",
        Duration::from_millis(100),
        Duration::from_millis(100),
        Duration::from_millis(100),
    ),
    LatencyBudget::new(
        "resize_recreation",
        Duration::from_millis(250),
        Duration::from_millis(350),
        Duration::from_millis(350),
    ),
    LatencyBudget::new(
        "target_loss_recovery",
        Duration::from_millis(1_250),
        Duration::from_millis(1_250),
        Duration::from_millis(1_250),
    ),
    LatencyBudget::new(
        "close_drain",
        Duration::from_millis(10),
        Duration::from_millis(10),
        Duration::from_millis(10),
    ),
];

/// Maximum live Rust heap for the Windows 1280x720 production-capture profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_HEAP_LIMIT_BYTES: usize = 32 * 1_024 * 1_024;

/// Maximum live Rust heap for the Windows 1280x720 transition profile.
pub const PHASE2_WINDOWS_PRODUCTION_TRANSITION_1280_HEAP_LIMIT_BYTES: usize = 32 * 1_024 * 1_024;

/// Maximum resident high-water mark for either Windows 1280x720 profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_RESIDENT_LIMIT_BYTES: u64 = 256 * 1_024 * 1_024;

/// Maximum callback-copy bytes retained by one Windows 1280x720 sample.
pub const PHASE2_WINDOWS_PRODUCTION_1280_COPIED_BYTES_LIMIT: u64 = 1_280 * 720 * 4;

/// Maximum live detached textures in the Windows 1280x720 capture profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_DETACHED_TEXTURES_LIMIT: u64 = 2;

/// Maximum live staging textures in the Windows 1280x720 capture profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_STAGING_TEXTURES_LIMIT: u64 = 1;

/// Maximum live producer, detached, and staging textures in that profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_GPU_RESOURCES_LIMIT: u64 = 5;

/// Maximum sustained stale work in the Windows 1280x720 capture profile.
pub const PHASE2_WINDOWS_PRODUCTION_1280_STALE_WORK_LIMIT: f64 = 0.02;

/// Phase 2 Windows dual-4K production-capture ceilings accepted by ADR 0032.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_LATENCY_BUDGETS: [LatencyBudget; 3] = [
    LatencyBudget::new(
        "dual_display_frame_arrival",
        Duration::from_millis(75),
        Duration::from_millis(150),
        Duration::from_millis(200),
    ),
    LatencyBudget::new(
        "dual_display_callback_copy",
        Duration::from_micros(200),
        Duration::from_micros(500),
        Duration::from_micros(1_500),
    ),
    LatencyBudget::new(
        "dual_display_moving_seam",
        Duration::from_millis(125),
        Duration::from_millis(175),
        Duration::from_millis(225),
    ),
];

/// Maximum live Rust heap for the Windows dual-4K production profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_HEAP_LIMIT_BYTES: usize = 384 * 1_024 * 1_024;

/// Maximum resident high-water mark for the Windows dual-4K production profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_RESIDENT_LIMIT_BYTES: u64 = 1_024 * 1_024 * 1_024;

/// Maximum callback-copy bytes retained by one Windows dual-4K sample.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_COPIED_BYTES_LIMIT: u64 = 3_840 * 2_160 * 4 * 6;

/// Maximum live detached textures in the Windows dual-4K production profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_DETACHED_TEXTURES_LIMIT: u64 = 10;

/// Maximum live staging textures in the Windows dual-4K production profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STAGING_TEXTURES_LIMIT: u64 = 1;

/// Maximum live producer, detached, and staging textures in that profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_GPU_RESOURCES_LIMIT: u64 = 15;

/// Maximum sustained stale work in the Windows dual-4K production profile.
pub const PHASE2_WINDOWS_PRODUCTION_DUAL_4K_STALE_WORK_LIMIT: f64 = 0.75;

const fn phase2_2_process_latency_budgets(event_p95: Duration) -> [LatencyBudget; 5] {
    [
        LatencyBudget::new(
            "discovery_open_retained_authority",
            Duration::from_millis(350),
            Duration::from_millis(750),
            Duration::from_secs(2),
        ),
        LatencyBudget::new(
            "event_authority_preflight_post",
            event_p95,
            event_p95,
            Duration::from_secs(2),
        ),
        LatencyBudget::new(
            "release_cleanup",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_millis(250),
        ),
        LatencyBudget::new(
            "session_close",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_secs(1),
        ),
        LatencyBudget::new(
            "fixture_controller_close",
            Duration::from_millis(100),
            Duration::from_millis(250),
            Duration::from_secs(1),
        ),
    ]
}

/// Phase 2.2 AppKit process-directed latency ceilings frozen before qualification.
pub const PHASE2_2_PROCESS_APPKIT_LATENCY_BUDGETS: [LatencyBudget; 5] =
    phase2_2_process_latency_budgets(Duration::from_micros(106_340));

/// Phase 2.2 controlled game-like process-directed latency ceilings.
pub const PHASE2_2_PROCESS_GAME_LIKE_LATENCY_BUDGETS: [LatencyBudget; 5] =
    phase2_2_process_latency_budgets(Duration::from_micros(112_180));

/// Phase 2.2 process-diagnostic latency ceilings frozen before qualification.
pub const PHASE2_2_PROCESS_DIAGNOSTIC_LATENCY_BUDGETS: [LatencyBudget; 4] = [
    LatencyBudget::new(
        "event_diagnostics_off",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostics_normal",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostics_debug",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
    LatencyBudget::new(
        "event_diagnostic_overflow",
        Duration::from_millis(300),
        Duration::from_millis(750),
        Duration::from_secs(2),
    ),
];

/// Frozen Phase 2.2 live-Rust-heap ceiling for every process-directed workload.
pub const PHASE2_2_PROCESS_HEAP_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Phase 3 Apple Silicon accepted-profile OCR inference ceilings from ADR 0037.
pub const PHASE3_APPLE_OCR_LATENCY_BUDGETS: [LatencyBudget; 3] = [
    LatencyBudget::new(
        "onnx_cpu_hud_full",
        Duration::from_millis(600),
        Duration::from_millis(750),
        Duration::from_millis(900),
    ),
    LatencyBudget::new(
        "onnx_cpu_hud_region",
        Duration::from_millis(375),
        Duration::from_millis(450),
        Duration::from_millis(600),
    ),
    LatencyBudget::new(
        "onnx_cpu_blank",
        Duration::from_millis(175),
        Duration::from_millis(210),
        Duration::from_millis(300),
    ),
];

/// Maximum live Rust heap attributable to any Apple Silicon OCR workload.
pub const PHASE3_APPLE_OCR_HEAP_LIMIT_BYTES: usize = 20 * 1024 * 1024;

/// Maximum process resident high-water for the Apple Silicon OCR profile.
pub const PHASE3_APPLE_OCR_RESIDENT_LIMIT_BYTES: u64 = 768 * 1024 * 1024;

/// Maximum accepted default model validation and session-pair startup.
pub const PHASE3_APPLE_OCR_COLD_LOAD_LIMIT: Duration = Duration::from_millis(175);

/// Maximum first close after the complete Apple Silicon OCR workload set.
pub const PHASE3_APPLE_OCR_CLOSE_LIMIT: Duration = Duration::from_millis(2);

/// Maximum accepted-model reopen and close cycle.
pub const PHASE3_APPLE_OCR_REOPEN_CLOSE_LIMIT: Duration = Duration::from_millis(100);

/// Accepted backend input-tensor byte ceiling recorded by the OCR profile.
pub const PHASE3_OCR_MAX_TENSOR_BYTES: u64 = 256 * 1024 * 1024;

/// Accepted backend native-output byte ceiling recorded by the OCR profile.
pub const PHASE3_OCR_MAX_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;

/// Exact mapped BGRA bytes for one complete 960 by 540 HUD frame.
pub const PHASE3_OCR_FULL_MAPPED_BYTES: u64 = 960 * 540 * 4;

/// Exact mapped BGRA bytes for the accepted 180 by 90 bounded HUD region.
pub const PHASE3_OCR_REGION_MAPPED_BYTES: u64 = 180 * 90 * 4;

/// Exact mapped BGRA bytes for the accepted 64 by 64 empty frame.
pub const PHASE3_OCR_EMPTY_MAPPED_BYTES: u64 = 64 * 64 * 4;

/// Enforces frozen p50, p95, and per-scenario latency ceilings.
///
/// A missing or duplicated workload is a harness error rather than a skipped
/// gate. The hard maximum is checked independently because a passing percentile
/// must never conceal one operation that escaped its scenario bound.
///
/// # Panics
///
/// Panics when a budget is malformed, names anything other than one measured
/// workload, or when a retained measurement exceeds any ceiling.
pub fn enforce_latency_budgets(workloads: &[Workload], budgets: &[LatencyBudget]) {
    for (index, budget) in budgets.iter().enumerate() {
        assert!(
            budget.p50 <= budget.p95 && budget.p95 <= budget.hard_max,
            "latency budget for {} must satisfy p50 <= p95 <= hard maximum",
            budget.workload
        );
        assert!(
            budgets[..index]
                .iter()
                .all(|earlier| earlier.workload != budget.workload),
            "latency budget for {} is duplicated",
            budget.workload
        );
        let mut matching = workloads
            .iter()
            .filter(|workload| workload.name() == budget.workload);
        let workload = matching.next().unwrap_or_else(|| {
            panic!(
                "latency budget names unmeasured workload {}",
                budget.workload
            )
        });
        assert!(
            matching.next().is_none(),
            "measured workload {} is duplicated",
            budget.workload
        );

        let p50 = workload.percentile(0.50);
        let p95 = workload.percentile(0.95);
        let hard_max = workload.max_elapsed();
        assert!(
            p50 <= budget.p50.as_secs_f64() * 1_000.0,
            "{} exceeded frozen p50 latency ceiling: {p50:.6} ms > {:.6} ms",
            budget.workload,
            budget.p50.as_secs_f64() * 1_000.0
        );
        assert!(
            p95 <= budget.p95.as_secs_f64() * 1_000.0,
            "{} exceeded frozen p95 latency ceiling: {p95:.6} ms > {:.6} ms",
            budget.workload,
            budget.p95.as_secs_f64() * 1_000.0
        );
        assert!(
            hard_max <= budget.hard_max,
            "{} exceeded frozen hard scenario bound: {:.6} ms > {:.6} ms",
            budget.workload,
            hard_max.as_secs_f64() * 1_000.0,
            budget.hard_max.as_secs_f64() * 1_000.0
        );
    }
}

/// Requires one present nonzero observation at or below an accepted ceiling.
///
/// # Panics
///
/// Panics when the observation is absent, zero, or exceeds `limit`.
#[track_caller]
pub fn nonzero_at_most(name: &str, observed: Option<u64>, limit: u64) -> u64 {
    let observed =
        observed.unwrap_or_else(|| panic!("{name} must report one observed resource count"));
    assert!(observed > 0, "{name} must report a nonzero resource count");
    assert!(
        observed <= limit,
        "{name} exceeded its accepted upper bound: {observed} > {limit}"
    );
    observed
}

// --- Hard budgets --------------------------------------------------------------

/// The `kind = "hard"` predicates every committed profile states, enforced by
/// [`enforce_hard_budgets`].
///
/// Copied rather than parsed. Reading them out of the profiles would need a
/// TOML reader and an evaluator for the predicate expression, which is a
/// dependency and a small language for a set of two strings. What keeps the
/// copies honest is `tests/hard_budget_drift.rs`, which reads all four
/// committed profiles and fails when the hard predicates they state are not
/// exactly these.
pub const HARD_BUDGET_PREDICATES: [&str; 2] =
    ["result_correctness == 0", "allocated_growth_bytes <= 4096"];

/// The bound in the second predicate above, as the number it is compared with.
pub const GROWTH_LIMIT_BYTES: i64 = 4096;

/// Fails the run when a workload violates a hard budget.
///
/// Every in-process benchmark target calls this unconditionally, so the two
/// predicates are enforced on the `cargo bench` path and on the
/// `cargo test --all-targets` path that CI runs on both release targets. Call it
/// after the report, so a run that fails still emits the numbers that explain it.
///
/// Sensitivity differs between the two paths even though the gate does not. A
/// smoke run retains three samples ([`Plan::smoke`]), so a per-iteration leak
/// has three iterations to exceed one page rather than the two hundred a
/// `--bench` run gives it. A leak is still a leak on both paths; what belongs
/// to the `--bench` run alone is the claim that a leak of a few dozen bytes per
/// iteration is caught.
///
/// # Panics
///
/// Panics naming the workload, the predicate it violated, and the measurement
/// that violated it.
pub fn enforce_hard_budgets(workloads: &[Workload]) {
    let [_correctness, growth] = HARD_BUDGET_PREDICATES;
    enforce_correctness(workloads);

    for workload in workloads {
        assert!(
            workload.growth_bytes <= GROWTH_LIMIT_BYTES,
            "{}: {growth} — live heap grew {} bytes over {} retained samples, \
             so a repeated operation did not give back what it took",
            workload.name,
            workload.growth_bytes,
            workload.elapsed.len(),
        );
    }
}

/// Fails when any retained sample violates its workload oracle.
///
/// Native evidence uses this before its measured bounded-growth predicate has
/// been set from both release targets. Phase 1 calls [`enforce_hard_budgets`],
/// which applies this same rule and its established growth bound together.
///
/// # Panics
///
/// Panics naming the workload and oracle when a retained sample is incorrect.
pub fn enforce_correctness(workloads: &[Workload]) {
    let correctness = HARD_BUDGET_PREDICATES[0];
    for workload in workloads {
        assert!(
            workload.incorrect == 0,
            "{}: {correctness} — {} of {} retained samples produced an output \
             its oracle rejected ({})",
            workload.name,
            workload.incorrect,
            workload.elapsed.len(),
            workload.oracle,
        );
    }
}

/// Classification of one line emitted by a native benchmark fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixedLineMatch {
    /// The line is unrelated to the observation family and may be skipped.
    Irrelevant,
    /// The line is the exact observation the current sample expects.
    Expected,
    /// The line belongs to the observation family but names a different outcome.
    Unexpected,
}

/// Classifies an exact fixture observation without discarding sibling outcomes.
///
/// Native benchmark readers may ignore readiness and control records, but once a
/// line belongs to the supplied observation family, any non-exact value is an
/// oracle failure rather than noise.
#[must_use]
pub fn classify_prefixed_line(
    line: &str,
    observation_prefix: &str,
    expected: &str,
) -> PrefixedLineMatch {
    if !line.starts_with(observation_prefix) {
        PrefixedLineMatch::Irrelevant
    } else if line == expected {
        PrefixedLineMatch::Expected
    } else {
        PrefixedLineMatch::Unexpected
    }
}
/// Returns the value of a `--name value` or `--name=value` argument.
#[must_use]
pub fn argument(arguments: &[String], name: &str) -> Option<String> {
    let mut iterator = arguments.iter();
    let prefix = format!("{name}=");
    while let Some(argument) = iterator.next() {
        if argument == name {
            return iterator.next().cloned();
        }
        if let Some(value) = argument.strip_prefix(&prefix) {
            return Some(value.to_owned());
        }
    }

    None
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::{
        ChildContainment, ChildExitObservation, LatencyBudget, PipeReaderEvent, PipeStream, Plan,
        PrefixedLineMatch, PrimaryChildCleanup, Sample, Workload, bounded_child_output,
        bounded_child_output_checked, bounded_child_output_with, classify_prefixed_line,
        enforce_latency_budgets, measure, measure_pair, nonzero_at_most, process_is_live,
        wait_for_child_exit,
    };
    use std::cell::Cell;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn fixture() {}

    const CHILD_MODE: &str = "MADO_PILOT_TESTKIT_BOUNDED_CHILD_MODE";
    const DESCENDANT_PID_REPORT: &str = "MADO_PILOT_TESTKIT_DESCENDANT_PID_REPORT";
    static PID_REPORT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct PidReport {
        path: PathBuf,
    }

    impl PidReport {
        fn new() -> Self {
            let sequence = PID_REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            Self {
                path: std::env::temp_dir().join(format!(
                    "mado-pilot-bounded-child-{}-{sequence}.pid",
                    std::process::id()
                )),
            }
        }

        fn command(&self, mode: &str) -> Command {
            let mut command = child_command(mode);
            command.env(DESCENDANT_PID_REPORT, &self.path);
            command
        }

        fn descendant_pid(&self) -> u32 {
            fs::read_to_string(&self.path)
                .expect("the fixture records its descendant PID")
                .lines()
                .find_map(|line| {
                    line.strip_prefix("descendant ")
                        .and_then(|pid| pid.parse().ok())
                })
                .expect("the PID report names the descendant")
        }
    }

    impl Drop for PidReport {
        fn drop(&mut self) {
            let _removed = fs::remove_file(&self.path);
        }
    }

    fn child_command(mode: &str) -> Command {
        let mut command =
            Command::new(std::env::current_exe().expect("the current test executable exists"));
        command
            .args(["bounded_child_fixture", "--nocapture"])
            .env(CHILD_MODE, mode);
        command
    }

    fn record_descendant_pid(descendant: &Child) {
        if let Some(path) = std::env::var_os(DESCENDANT_PID_REPORT) {
            let mut report = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("the fixture opens its descendant PID report");
            writeln!(report, "descendant {}", descendant.id())
                .expect("the fixture records its descendant PID");
            report
                .flush()
                .expect("the fixture flushes its descendant PID report");
        }
    }

    fn spawn_pipe_holding_descendant() {
        let descendant = child_command("timeout")
            .spawn()
            .expect("the bounded fixture spawns its descendant");
        record_descendant_pid(&descendant);
        drop(descendant);
    }

    fn spawn_closed_pipe_descendant() {
        let descendant = child_command("timeout")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the bounded fixture spawns its closed-pipe descendant");
        record_descendant_pid(&descendant);
        drop(descendant);
    }

    struct ObservedPrimaryCleanup {
        defer_termination: bool,
        completion: Option<mpsc::Sender<()>>,
        reader_events: Option<mpsc::Sender<PipeReaderEvent>>,
    }

    impl PrimaryChildCleanup for ObservedPrimaryCleanup {
        fn terminate_and_wait(
            &mut self,
            containment: &mut ChildContainment,
            child: &mut Child,
            deadline: Instant,
        ) -> ChildExitObservation {
            if self.defer_termination {
                return ChildExitObservation::default();
            }
            containment.terminate(Some(child));
            wait_for_child_exit(child, deadline)
        }

        fn reaper_completion(&mut self) -> Option<mpsc::Sender<()>> {
            self.completion.take()
        }

        fn reader_events(&mut self) -> Option<mpsc::Sender<PipeReaderEvent>> {
            self.reader_events.take()
        }
    }

    fn observed_primary_cleanup(
        defer_termination: bool,
    ) -> (
        ObservedPrimaryCleanup,
        mpsc::Receiver<()>,
        mpsc::Receiver<PipeReaderEvent>,
    ) {
        let (completion, completed) = mpsc::channel();
        let (reader_events, observed_reader_events) = mpsc::channel();
        (
            ObservedPrimaryCleanup {
                defer_termination,
                completion: Some(completion),
                reader_events: Some(reader_events),
            },
            completed,
            observed_reader_events,
        )
    }

    fn assert_readers_reached_eof_and_were_joined(reader_events: &mpsc::Receiver<PipeReaderEvent>) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut observed = Vec::with_capacity(4);
        while observed.len() < 4 {
            observed.push(
                reader_events
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .expect("both pipe readers reach EOF and are joined"),
            );
        }
        for expected in [
            PipeReaderEvent::Eof(PipeStream::Stdout),
            PipeReaderEvent::Eof(PipeStream::Stderr),
            PipeReaderEvent::Joined(PipeStream::Stdout),
            PipeReaderEvent::Joined(PipeStream::Stderr),
        ] {
            assert!(
                observed.contains(&expected),
                "missing independent reader lifecycle event {expected:?}: {observed:?}"
            );
        }
    }

    fn assert_process_is_reaped(process_id: u32, role: &str) {
        assert_ne!(process_id, 0, "{role} PID must be observable");
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_is_live(process_id) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !process_is_live(process_id),
            "{role} process {process_id} remained live after cleanup"
        );
    }

    #[test]
    fn bounded_child_fixture() {
        match std::env::var(CHILD_MODE).as_deref() {
            Ok("success") => println!("bounded child completed"),
            Ok("overflow") => {
                let mut stdout = std::io::stdout().lock();
                stdout
                    .write_all(&vec![b'x'; 4_096])
                    .expect("the child writes its oversized output");
                stdout.flush().expect("the child flushes stdout");
            }
            Ok("timeout") => std::thread::sleep(Duration::from_secs(5)),
            Ok("descendant-timeout") => {
                spawn_pipe_holding_descendant();
                std::thread::sleep(Duration::from_secs(5));
            }
            Ok("held-pipe") => spawn_pipe_holding_descendant(),
            Ok("closed-pipe-descendant") => spawn_closed_pipe_descendant(),
            _ => {}
        }
    }

    #[test]
    fn bounded_child_output_accepts_a_timely_finite_process() {
        let output = bounded_child_output(
            &mut child_command("success"),
            Duration::from_secs(5),
            16 * 1_024,
        );

        assert!(output.within_bounds);
        assert!(output.status.is_some_and(|status| status.success()));
        assert!(
            String::from_utf8(output.stdout)
                .expect("the child writes UTF-8")
                .contains("bounded child completed")
        );
    }

    #[test]
    fn bounded_child_output_rejects_an_unrecognized_live_process() {
        let output = bounded_child_output_checked(
            &mut child_command("success"),
            Duration::from_secs(1),
            16 * 1_024,
            |_process_id| false,
        );

        assert!(!output.within_bounds);
        assert!(
            output.status.is_some(),
            "a rejected live process remains owned until it is reaped"
        );
    }

    #[test]
    fn bounded_child_output_reaps_a_process_after_timeout() {
        let started = Instant::now();
        let output = bounded_child_output(
            &mut child_command("timeout"),
            Duration::from_millis(25),
            16 * 1_024,
        );

        assert!(!output.within_bounds);
        assert!(
            output.status.is_some(),
            "the terminated child was not reaped"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the timed-out child exceeded the bounded termination allowance"
        );
    }

    #[test]
    fn bounded_child_output_transfers_a_child_that_primary_cleanup_cannot_reap() {
        let (mut primary_cleanup, cleanup_finished, reader_events) = observed_primary_cleanup(true);
        let child_pid = Cell::new(0);
        let started = Instant::now();
        let output = bounded_child_output_with(
            &mut child_command("timeout"),
            Duration::from_millis(25),
            16 * 1_024,
            &mut primary_cleanup,
            |process_id| {
                child_pid.set(process_id);
                true
            },
        );
        let returned_after = started.elapsed();

        assert!(!output.within_bounds);
        assert!(
            output.status.is_none(),
            "the injected failed termination allowance cannot report a status"
        );
        assert!(
            returned_after < Duration::from_secs(2),
            "the unreaped child blocked the bounded primary result"
        );
        assert_readers_reached_eof_and_were_joined(&reader_events);
        assert_process_is_reaped(child_pid.get(), "timed-out child");
        cleanup_finished
            .recv_timeout(Duration::from_secs(2))
            .expect("the dedicated reaper completes retained cleanup");
    }

    #[test]
    fn bounded_child_output_reaps_a_timed_out_child_and_its_descendant() {
        let report = PidReport::new();
        let mut command = report.command("descendant-timeout");
        let (mut primary_cleanup, cleanup_finished, reader_events) = observed_primary_cleanup(true);
        let child_pid = Cell::new(0);
        let started = Instant::now();
        let output = bounded_child_output_with(
            &mut command,
            Duration::from_secs(1),
            16 * 1_024,
            &mut primary_cleanup,
            |process_id| {
                child_pid.set(process_id);
                true
            },
        );
        let returned_after = started.elapsed();
        let descendant_pid = report.descendant_pid();

        assert!(!output.within_bounds);
        assert!(output.status.is_none());
        assert!(
            returned_after < Duration::from_secs(2),
            "the descendant blocked the bounded primary result"
        );
        assert_readers_reached_eof_and_were_joined(&reader_events);
        assert_process_is_reaped(child_pid.get(), "timed-out child");
        assert_process_is_reaped(descendant_pid, "timed-out descendant");
        cleanup_finished
            .recv_timeout(Duration::from_secs(2))
            .expect("tree containment cleanup completes");
    }

    #[test]
    fn bounded_child_output_keeps_readers_owned_while_a_descendant_holds_the_pipes() {
        let report = PidReport::new();
        let mut command = report.command("held-pipe");
        let (mut primary_cleanup, cleanup_finished, reader_events) =
            observed_primary_cleanup(false);
        let child_pid = Cell::new(0);
        let started = Instant::now();
        let output = bounded_child_output_with(
            &mut command,
            Duration::from_secs(1),
            16 * 1_024,
            &mut primary_cleanup,
            |process_id| {
                child_pid.set(process_id);
                true
            },
        );
        let returned_after = started.elapsed();
        let descendant_pid = report.descendant_pid();

        assert!(!output.within_bounds);
        assert!(output.status.is_none_or(|status| status.success()));
        assert!(
            returned_after < Duration::from_secs(2),
            "a descendant-held pipe blocked the bounded primary result"
        );
        assert_readers_reached_eof_and_were_joined(&reader_events);
        assert_process_is_reaped(child_pid.get(), "finite child");
        assert_process_is_reaped(descendant_pid, "pipe-holding descendant");
        cleanup_finished
            .recv_timeout(Duration::from_secs(2))
            .expect("reader-retaining cleanup completes");
    }

    #[test]
    fn bounded_child_output_contains_a_closed_pipe_descendant_before_return() {
        let report = PidReport::new();
        let mut command = report.command("closed-pipe-descendant");
        let (mut primary_cleanup, _cleanup_finished, reader_events) =
            observed_primary_cleanup(false);
        let child_pid = Cell::new(0);
        let output = bounded_child_output_with(
            &mut command,
            Duration::from_secs(1),
            16 * 1_024,
            &mut primary_cleanup,
            |process_id| {
                child_pid.set(process_id);
                true
            },
        );
        let descendant_pid = report.descendant_pid();

        #[cfg(not(windows))]
        assert!(output.within_bounds, "{output:?}");
        #[cfg(windows)]
        assert!(
            !output.within_bounds,
            "hosted Windows keeps a pipe reader pending until job containment terminates the \
             descendant, so the primary result must stay conservative: {output:?}"
        );
        assert!(output.status.is_some_and(|status| status.success()));
        assert_readers_reached_eof_and_were_joined(&reader_events);
        assert_process_is_reaped(child_pid.get(), "finite child");
        assert_process_is_reaped(descendant_pid, "closed-pipe descendant");
    }

    #[test]
    fn bounded_child_output_rejects_and_caps_oversized_stdout() {
        let output = bounded_child_output(
            &mut child_command("overflow"),
            Duration::from_secs(1),
            1_024,
        );

        assert!(!output.within_bounds);
        assert!(output.status.is_some_and(|status| status.success()));
        assert_eq!(output.stdout.len(), 1_024);
    }

    fn stale_sample(_: &()) -> Sample {
        Sample::unmapped(Duration::from_micros(1), true).with_stale_work(1, 4)
    }

    #[test]
    fn an_observable_stale_ratio_keeps_its_denominator() {
        let workload = measure(
            "stale",
            "one of four publications is skipped",
            Plan::new(0, 2),
            fixture,
            stale_sample,
        );

        assert_eq!(workload.stale_work_ratio(), Some(0.25));
    }

    struct PairedFixture {
        iterations: Cell<u64>,
    }

    fn paired_sample(fixture: &PairedFixture) -> (Sample, Sample) {
        let iteration = fixture.iterations.get() + 1;
        fixture.iterations.set(iteration);
        (
            Sample::unmapped(Duration::from_millis(iteration), true),
            Sample::unmapped(Duration::from_millis(iteration * 10), true),
        )
    }

    #[test]
    fn paired_measurement_invokes_one_operation_per_iteration() {
        let [first, second] = measure_pair(
            ("first", "the first timing view is retained"),
            ("second", "the second timing view is retained"),
            Plan::new(2, 3),
            || PairedFixture {
                iterations: Cell::new(0),
            },
            paired_sample,
        );

        assert_eq!(first.elapsed, [3, 4, 5].map(Duration::from_millis).to_vec());
        assert_eq!(
            second.elapsed,
            [30, 40, 50].map(Duration::from_millis).to_vec()
        );
        assert_eq!(first.warmup_iterations, 2);
        assert_eq!(second.warmup_iterations, 2);
        assert_eq!(first.sample_count, 3);
        assert_eq!(second.sample_count, 3);
    }

    fn timed_workload(name: &'static str, elapsed: Vec<Duration>) -> Workload {
        Workload {
            name,
            oracle: "the synthetic timing sample is accepted",
            warmup_iterations: 0,
            sample_count: 1,
            elapsed,
            incorrect: 0,
            stale: 0,
            scheduled: 0,
            mapped: 0,
            iteration_span: Duration::ZERO,
            peak_bytes: 0,
            steady_bytes: 0,
            peak_resident_bytes: None,
            copied_bytes: None,
            detached_textures_peak: None,
            staging_textures_peak: None,
            gpu_resources_peak: None,
            growth_bytes: 0,
        }
    }

    #[test]
    fn frozen_latency_budgets_check_percentiles_and_the_slowest_sample() {
        let workloads = [timed_workload(
            "qualified",
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(30),
            ],
        )];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(20),
                Duration::from_millis(30),
                Duration::from_millis(40),
            )],
        );
    }

    #[test]
    #[should_panic(expected = "exceeded frozen hard scenario bound")]
    fn one_slow_sample_cannot_hide_behind_a_passing_percentile() {
        let mut elapsed = vec![Duration::from_millis(1); 100];
        elapsed.push(Duration::from_millis(501));
        let workloads = [timed_workload("qualified", elapsed)];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(10),
                Duration::from_millis(10),
                Duration::from_millis(500),
            )],
        );
    }

    #[test]
    #[should_panic(expected = "exceeded frozen p95 latency ceiling")]
    fn a_percentile_ceiling_is_not_treated_as_only_a_hard_maximum() {
        let workloads = [timed_workload(
            "qualified",
            vec![
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(30),
            ],
        )];

        enforce_latency_budgets(
            &workloads,
            &[LatencyBudget::new(
                "qualified",
                Duration::from_millis(2),
                Duration::from_millis(20),
                Duration::from_millis(40),
            )],
        );
    }

    #[test]
    fn nonzero_upper_bounds_accept_below_and_equal_observations() {
        assert_eq!(nonzero_at_most("resource", Some(1), 2), 1);
        assert_eq!(nonzero_at_most("resource", Some(2), 2), 2);
    }

    #[test]
    #[should_panic(expected = "exceeded its accepted upper bound")]
    fn nonzero_upper_bounds_reject_an_above_limit_observation() {
        let _observed = nonzero_at_most("resource", Some(3), 2);
    }

    #[test]
    #[should_panic(expected = "must report a nonzero resource count")]
    fn nonzero_upper_bounds_reject_zero() {
        let _observed = nonzero_at_most("resource", Some(0), 2);
    }

    #[test]
    #[should_panic(expected = "must report one observed resource count")]
    fn nonzero_upper_bounds_reject_a_missing_observation() {
        let _observed = nonzero_at_most("resource", None, 2);
    }

    #[test]
    fn a_wrong_role_observation_is_not_skipped_before_the_expected_target() {
        assert_eq!(
            classify_prefixed_line(
                "control queue-block=ready",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Irrelevant
        );
        assert_eq!(
            classify_prefixed_line(
                "observation role=sibling family=pointer-move units=1",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Unexpected
        );
        assert_eq!(
            classify_prefixed_line(
                "observation role=target family=pointer-move units=1",
                "observation role=",
                "observation role=target family=pointer-move units=1",
            ),
            PrefixedLineMatch::Expected
        );
    }
    #[test]
    fn percentile_uses_nearest_rank_for_even_sample_counts() {
        let workload = Workload {
            name: "nearest-rank",
            oracle: "the selected order statistic is exact",
            warmup_iterations: 0,
            sample_count: 50,
            elapsed: (1..=50).map(Duration::from_millis).collect(),
            incorrect: 0,
            stale: 0,
            scheduled: 0,
            mapped: 0,
            iteration_span: Duration::ZERO,
            peak_bytes: 0,
            steady_bytes: 0,
            peak_resident_bytes: None,
            copied_bytes: None,
            detached_textures_peak: None,
            staging_textures_peak: None,
            gpu_resources_peak: None,
            growth_bytes: 0,
        };

        assert_eq!(workload.percentile(0.50), 25.0);
        assert_eq!(workload.percentile(0.95), 48.0);
        assert_eq!(workload.percentile(1.0), 50.0);
    }
}
