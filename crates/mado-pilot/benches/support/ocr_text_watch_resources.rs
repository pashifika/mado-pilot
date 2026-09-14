//! Current and process-high-water OS observations, never substituted with zero.

#[derive(Debug)]
pub(super) struct Resident {
    pub current: Option<u64>,
    pub peak: Option<u64>,
    pub private: Option<u64>,
    pub footprint: Option<u64>,
}

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub(super) fn resident() -> Resident {
    // TASK_VM_INFO_REV1 is a stable prefix: virtual size, two integer_t values,
    // then 17 mach_vm_size_t values through phys_footprint (38 natural_t words).
    // Using the prefix permits old supported kernels without a new dependency.
    let mut words = [0_u64; 19];
    let mut count = 38;
    #[allow(
        deprecated,
        reason = "the existing libc dev dependency exposes this Mach self-task API"
    )]
    // SAFETY: self-task is process-owned; words is aligned writable storage for
    // the exact 152-byte rev1 prefix. count is in 32-bit natural_t words, and
    // task_info may initialize only that declared extent. No pointer is retained.
    let status = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            22,
            words.as_mut_ptr().cast(),
            &raw mut count,
        )
    };
    let available = status == 0 && count >= 38;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: RUSAGE_SELF writes one complete rusage into valid writable storage.
    let peak = if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        // SAFETY: successful getrusage initialized the complete rusage.
        u64::try_from(unsafe { usage.assume_init() }.ru_maxrss)
            .ok()
            .filter(|value| *value > 0)
    } else {
        None
    };
    Resident {
        current: available.then_some(words[2]).filter(|value| *value > 0),
        peak,
        private: None,
        footprint: available.then_some(words[18]).filter(|value| *value > 0),
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"))]
pub(super) fn resident() -> Resident {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size =
        u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS_EX>()).expect("fixed structure size");
    counters.cb = size;
    // SAFETY: the extended structure starts with PROCESS_MEMORY_COUNTERS; cb and
    // the explicit byte count include the writable PrivateUsage suffix. The
    // pseudo-handle needs no close and Windows retains neither pointer.
    let available =
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), size) }
            .is_ok();
    Resident {
        current: available
            .then_some(counters.WorkingSetSize as u64)
            .filter(|value| *value > 0),
        peak: available
            .then_some(counters.PeakWorkingSetSize as u64)
            .filter(|value| *value > 0),
        private: available.then_some(counters.PrivateUsage as u64),
        footprint: None,
    }
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_os = "macos"),
    all(target_arch = "x86_64", target_os = "windows", target_env = "msvc"),
)))]
pub(super) fn resident() -> Resident {
    Resident {
        current: None,
        peak: None,
        private: None,
        footprint: None,
    }
}
