//! Private retained-window identity and bounded Win32 revalidation.

use std::ffi::c_void;
use std::fmt;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetClassNameW, GetWindowThreadProcessId, IsWindow,
};

use crate::discovery::NativeKey;

const CLASS_CAPACITY: usize = 256;
const STILL_ACTIVE_EXIT_CODE: u32 = 259;

/// The privacy-safe result of comparing current native facts with a retained target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowAuthorityStatus {
    SameTarget,
    TargetLost,
    ReplacementOrReuse,
    RelationshipChanged,
    Unavailable,
}

/// Discovery-time authority retained for one top-level window.
pub(crate) struct RetainedWindowAuthority {
    hwnd: usize,
    owner: OwnedProcessHandle,
    identity: WindowIdentity,
}

impl RetainedWindowAuthority {
    /// Captures authority only when every required fact is available and stable.
    pub(crate) fn capture(key: NativeKey, expected_class: Option<&str>) -> Option<Self> {
        let NativeKey::Window(hwnd) = key else {
            return None;
        };
        let expected_class = WindowClass::from_str(expected_class?)?;
        let observed = observe(hwnd).ok()?;
        if observed.identity.root != hwnd || observed.identity.class != expected_class {
            return None;
        }
        Some(Self {
            hwnd,
            owner: observed.owner,
            identity: observed.identity,
        })
    }

    /// Re-reads all identity facts once and classifies the current target.
    pub(crate) fn status(&self) -> WindowAuthorityStatus {
        match owner_is_active(self.owner.handle()) {
            Ok(true) => classify(
                self.hwnd,
                self.identity,
                observe(self.hwnd).map(|observation| observation.identity),
            ),
            Ok(false) => WindowAuthorityStatus::TargetLost,
            Err(()) => WindowAuthorityStatus::Unavailable,
        }
    }
}

impl fmt::Debug for RetainedWindowAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedWindowAuthority")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowIdentity {
    owner_process: u32,
    owner_creation: u64,
    owner_thread: u32,
    root: usize,
    class: WindowClass,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowFacts {
    owner_process: u32,
    owner_thread: u32,
    root: usize,
    class: WindowClass,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowClass {
    units: [u16; CLASS_CAPACITY],
    len: u16,
}

impl WindowClass {
    fn read(hwnd: HWND) -> Result<Self, ObservationFault> {
        let mut units = [0u16; CLASS_CAPACITY];
        // SAFETY: `units` is writable for the call and `hwnd` is treated as an
        // opaque value. The surrounding observation checks window liveness.
        let written = unsafe { GetClassNameW(hwnd, &mut units) };
        let len = u16::try_from(written).map_err(|_| ObservationFault::Unavailable)?;
        if len == 0 {
            return if is_window(hwnd) {
                Err(ObservationFault::Unavailable)
            } else {
                Err(ObservationFault::Lost)
            };
        }
        Ok(Self { units, len })
    }

    fn from_str(value: &str) -> Option<Self> {
        let mut units = [0u16; CLASS_CAPACITY];
        let mut len = 0usize;
        for unit in value.encode_utf16() {
            if len >= CLASS_CAPACITY - 1 {
                return None;
            }
            units[len] = unit;
            len += 1;
        }
        (len > 0).then_some(Self {
            units,
            len: u16::try_from(len).ok()?,
        })
    }
}

struct Observation {
    identity: WindowIdentity,
    owner: OwnedProcessHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservationFault {
    Lost,
    Unavailable,
}

fn observe(raw: usize) -> Result<Observation, ObservationFault> {
    let hwnd = hwnd(raw);
    let before = read_window_facts(hwnd)?;
    let owner = open_process(before.owner_process)?;
    if !owner_is_active(owner.handle()).map_err(|()| ObservationFault::Unavailable)? {
        return Err(ObservationFault::Lost);
    }
    let owner_creation = process_creation(owner.handle())?;
    let after = read_window_facts(hwnd)?;
    if before != after {
        return Err(ObservationFault::Unavailable);
    }
    Ok(Observation {
        identity: WindowIdentity {
            owner_process: before.owner_process,
            owner_creation,
            owner_thread: before.owner_thread,
            root: before.root,
            class: before.class,
        },
        owner,
    })
}

fn read_window_facts(hwnd: HWND) -> Result<WindowFacts, ObservationFault> {
    if !is_window(hwnd) {
        return Err(ObservationFault::Lost);
    }

    let mut owner_process = 0u32;
    // SAFETY: `owner_process` is writable and `hwnd` is an opaque Win32 handle.
    let owner_thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut owner_process)) };
    if owner_thread == 0 || owner_process == 0 {
        return if is_window(hwnd) {
            Err(ObservationFault::Unavailable)
        } else {
            Err(ObservationFault::Lost)
        };
    }

    // SAFETY: `hwnd` is an opaque value and GA_ROOT performs no caller-memory access.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.0.is_null() {
        return if is_window(hwnd) {
            Err(ObservationFault::Unavailable)
        } else {
            Err(ObservationFault::Lost)
        };
    }
    let class = WindowClass::read(hwnd)?;
    if !is_window(hwnd) {
        return Err(ObservationFault::Lost);
    }

    Ok(WindowFacts {
        owner_process,
        owner_thread,
        root: root.0.addr(),
        class,
    })
}

fn classify(
    hwnd: usize,
    expected: WindowIdentity,
    observed: Result<WindowIdentity, ObservationFault>,
) -> WindowAuthorityStatus {
    let observed = match observed {
        Ok(observed) => observed,
        Err(ObservationFault::Lost) => return WindowAuthorityStatus::TargetLost,
        Err(ObservationFault::Unavailable) => return WindowAuthorityStatus::Unavailable,
    };
    if observed.owner_process != expected.owner_process
        || observed.owner_creation != expected.owner_creation
        || observed.class != expected.class
    {
        return WindowAuthorityStatus::ReplacementOrReuse;
    }
    if observed.owner_thread != expected.owner_thread
        || observed.root != expected.root
        || observed.root != hwnd
    {
        return WindowAuthorityStatus::RelationshipChanged;
    }
    WindowAuthorityStatus::SameTarget
}

fn open_process(process: u32) -> Result<OwnedProcessHandle, ObservationFault> {
    // SAFETY: the numeric process id came directly from GetWindowThreadProcessId;
    // the returned owned handle is closed by `OwnedProcessHandle`.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }
        .map_err(|_| ObservationFault::Unavailable)?;
    Ok(OwnedProcessHandle::new(handle))
}

fn process_creation(handle: HANDLE) -> Result<u64, ObservationFault> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all four FILETIME outputs are complete writable values and `handle`
    // is an owned process handle with query permission.
    unsafe {
        GetProcessTimes(
            handle,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    }
    .map_err(|_| ObservationFault::Unavailable)?;
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

fn owner_is_active(handle: HANDLE) -> Result<bool, ()> {
    let mut exit_code = 0u32;
    // SAFETY: `exit_code` is writable and `handle` is the retained process handle.
    unsafe { GetExitCodeProcess(handle, &raw mut exit_code) }.map_err(|_| ())?;
    Ok(exit_code == STILL_ACTIVE_EXIT_CODE)
}

fn hwnd(raw: usize) -> HWND {
    HWND(std::ptr::with_exposed_provenance_mut::<c_void>(raw))
}

fn is_window(hwnd: HWND) -> bool {
    // SAFETY: IsWindow accepts an opaque HWND and dereferences no caller memory.
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

struct OwnedProcessHandle(usize);

impl OwnedProcessHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle.0.addr())
    }

    fn handle(&self) -> HANDLE {
        HANDLE(std::ptr::with_exposed_provenance_mut::<c_void>(self.0))
    }
}

impl Drop for OwnedProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns exactly one OpenProcess handle and closes it once.
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ObservationFault, OwnedProcessHandle, WindowAuthorityStatus, WindowClass, WindowIdentity,
        classify,
    };

    fn identity() -> WindowIdentity {
        WindowIdentity {
            owner_process: 11,
            owner_creation: 22,
            owner_thread: 33,
            root: 44,
            class: WindowClass::from_str("MadoPilotAuthorityTest").expect("bounded class"),
        }
    }

    fn observed(identity: WindowIdentity) -> Result<WindowIdentity, ObservationFault> {
        Ok(identity)
    }

    fn classify_identity(
        expected: WindowIdentity,
        current: Result<WindowIdentity, ObservationFault>,
    ) -> WindowAuthorityStatus {
        classify(expected.root, expected, current)
    }

    #[test]
    fn unchanged_identity_is_the_same_target() {
        let expected = identity();
        assert_eq!(
            classify_identity(expected, observed(expected)),
            WindowAuthorityStatus::SameTarget
        );
    }

    #[test]
    fn same_value_handle_with_new_owner_lifetime_is_reuse() {
        let expected = identity();
        let mut current = expected;
        current.owner_creation += 1;
        assert_eq!(
            classify_identity(expected, observed(current)),
            WindowAuthorityStatus::ReplacementOrReuse
        );
    }

    #[test]
    fn restarted_owner_or_changed_class_is_replacement() {
        let expected = identity();
        for current in [
            WindowIdentity {
                owner_process: expected.owner_process + 1,
                ..expected
            },
            WindowIdentity {
                class: WindowClass::from_str("Replacement").expect("bounded class"),
                ..expected
            },
        ] {
            assert_eq!(
                classify_identity(expected, observed(current)),
                WindowAuthorityStatus::ReplacementOrReuse
            );
        }
    }

    #[test]
    fn reparent_or_owner_thread_change_is_relationship_change() {
        let expected = identity();
        for current in [
            WindowIdentity {
                root: expected.root + 1,
                ..expected
            },
            WindowIdentity {
                owner_thread: expected.owner_thread + 1,
                ..expected
            },
        ] {
            assert_eq!(
                classify_identity(expected, observed(current)),
                WindowAuthorityStatus::RelationshipChanged
            );
        }
    }

    #[test]
    fn lost_and_unavailable_observations_stay_distinct() {
        let expected = identity();
        assert_eq!(
            classify_identity(expected, Err(ObservationFault::Lost)),
            WindowAuthorityStatus::TargetLost
        );
        assert_eq!(
            classify_identity(expected, Err(ObservationFault::Unavailable)),
            WindowAuthorityStatus::Unavailable
        );
    }

    #[test]
    fn debug_output_contains_no_native_identity() {
        let authority = super::RetainedWindowAuthority {
            hwnd: 44,
            owner: OwnedProcessHandle(0),
            identity: identity(),
        };
        assert_eq!(format!("{authority:?}"), "RetainedWindowAuthority { .. }");
        let _authority = std::mem::ManuallyDrop::new(authority);
    }
}
