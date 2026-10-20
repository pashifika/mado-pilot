//! Self-process CPU time; units do not imply accounting accuracy.

#[cfg(target_os = "macos")]
pub(super) fn process_cpu_ns() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: RUSAGE_SELF initializes one writable rusage; no pointer is retained.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: successful getrusage initialized the complete structure.
    let usage = unsafe { usage.assume_init() };
    let nanos = |value: libc::timeval| {
        let seconds = u64::try_from(value.tv_sec).ok()?;
        let micros = u64::try_from(value.tv_usec)
            .ok()
            .filter(|value| *value < 1_000_000)?;
        seconds
            .checked_mul(1_000_000_000)?
            .checked_add(micros * 1_000)
    };
    nanos(usage.ru_utime)?.checked_add(nanos(usage.ru_stime)?)
}

#[cfg(windows)]
pub(super) fn process_cpu_ns() -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the self-process pseudo-handle needs no close. Each output points
    // to a distinct writable FILETIME, and the API retains none of them.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    }
    .ok()?;
    let ticks =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    ticks(kernel).checked_add(ticks(user))?.checked_mul(100)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(super) fn process_cpu_ns() -> Option<u64> {
    None
}
