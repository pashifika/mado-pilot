//! Private controller-side transport ownership for the macOS qualification fixture.
//!
//! This module authenticates the connected fixture by effective user, canonical
//! executable path, process identifier, and audit token before a controller trusts
//! any protocol record. Its socket lives in a unique mode-0700 directory, and
//! teardown signals the audit-token-bound process lifetime rather than a reusable
//! numeric PID.

use std::ffi::{CStr, c_int, c_void};
use std::fmt;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SOL_LOCAL: c_int = 0;
const LOCAL_PEERPID: c_int = 0x002;
const LOCAL_PEERTOKEN: c_int = 0x006;
const SIGKILL: c_int = 9;
const SIGTERM: c_int = 15;
const PROC_PIDPATHINFO_MAXSIZE: usize = 4 * 1_024;
const SOCKET_DIRECTORY_ATTEMPTS: usize = 32;

static SOCKET_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUN_NONCE: Mutex<u64> = Mutex::new(0);

/// Issues one nonzero, process-wide, strictly newer fixture-run identity.
///
/// The lock joins every controller in this artifact into one ordering. The
/// wall clock seeds the sequence but never replaces a later value already
/// issued in this process.
pub fn next_fixture_run_nonce() -> Result<u64, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the fixture run clock was unavailable")?
        .as_nanos();
    let timestamp = u64::try_from(timestamp)
        .map_err(|_| "the fixture run clock exceeded its identity range")?
        .max(1);
    let mut latest = RUN_NONCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let next = if *latest == 0 {
        timestamp
    } else {
        timestamp.max(
            latest
                .checked_add(1)
                .ok_or_else(|| "the fixture run identity was exhausted".to_owned())?,
        )
    };
    *latest = next;
    Ok(next)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuditToken {
    values: [u32; 8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FixturePeerIdentity {
    effective_user_id: u32,
    process_id: u32,
    executable: PathBuf,
    audit_token: AuditToken,
}

/// One authenticated fixture process lifetime retained from its Unix peer.
#[derive(Debug, Clone, Copy)]
pub struct AuthenticatedFixtureProcess {
    process_id: u32,
    audit_token: AuditToken,
}

impl AuthenticatedFixtureProcess {
    /// Returns the authenticated process identifier used by fixture discovery.
    pub const fn process_id(self) -> u32 {
        self.process_id
    }

    /// Confirms that one discovered owner PID is this still-live authenticated
    /// peer lifetime. The audit-token lookup refuses a reused numeric PID.
    #[must_use]
    pub fn matches_live_owner(self, owner_process: i64) -> bool {
        if i64::from(self.process_id) != owner_process {
            return false;
        }
        let mut audit_token = self.audit_token;
        let mut executable = [0u8; PROC_PIDPATHINFO_MAXSIZE];
        // SAFETY: `audit_token` came from the authenticated Unix peer and the
        // fixed output buffer is writable for the declared capacity.
        let length = unsafe {
            proc_pidpath_audittoken(
                &raw mut audit_token,
                executable.as_mut_ptr().cast::<c_void>(),
                u32::try_from(executable.len()).expect("the fixed path capacity fits u32"),
            )
        };
        length > 0 && usize::try_from(length).is_ok_and(|length| length < executable.len())
    }

    /// Requests termination of this exact audit-token-bound process lifetime.
    pub fn terminate(&mut self) -> bool {
        // SAFETY: the token was obtained from the authenticated Unix peer.
        // `proc_signal_with_audittoken` binds the signal to that exact process
        // lifetime rather than a subsequently reused numeric PID.
        unsafe { proc_signal_with_audittoken(&raw mut self.audit_token, SIGTERM) == 0 }
    }

    /// Forces termination of this exact audit-token-bound process lifetime.
    pub fn kill(&mut self) -> bool {
        // SAFETY: the token was obtained from the authenticated Unix peer.
        // `proc_signal_with_audittoken` binds the signal to that exact process
        // lifetime rather than a subsequently reused numeric PID.
        unsafe { proc_signal_with_audittoken(&raw mut self.audit_token, SIGKILL) == 0 }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) const fn for_test(process_id: u32) -> Self {
        Self {
            process_id,
            audit_token: AuditToken { values: [0; 8] },
        }
    }
}

/// Opaque comparison token for one foreground-process lifetime and physical
/// cursor observation.
///
/// The values are intentionally inaccessible: qualification may compare two
/// observations but cannot print another application's identity or cursor
/// coordinates into retained evidence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DesktopInputState {
    process: i64,
    process_launch_time: u64,
    pointer_x: u64,
    pointer_y: u64,
}

impl fmt::Debug for DesktopInputState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopInputState")
            .finish_non_exhaustive()
    }
}

/// Samples the current foreground-process lifetime and physical cursor without
/// prompting or exposing either value.
pub fn desktop_input_state() -> Result<DesktopInputState, String> {
    let mut process = 0i64;
    let mut process_launch_time = 0.0f64;
    let mut pointer_x = 0.0f64;
    let mut pointer_y = 0.0f64;
    // SAFETY: every output is writable for its declared scalar type.
    let status = unsafe {
        mp_shim_input_environment(
            &raw mut process,
            &raw mut process_launch_time,
            &raw mut pointer_x,
            &raw mut pointer_y,
        )
    };
    if status != 0
        || process <= 0
        || !process_launch_time.is_finite()
        || !pointer_x.is_finite()
        || !pointer_y.is_finite()
    {
        return Err("the desktop input state could not be observed".to_owned());
    }
    Ok(DesktopInputState {
        process,
        process_launch_time: process_launch_time.to_bits(),
        pointer_x: pointer_x.to_bits(),
        pointer_y: pointer_y.to_bits(),
    })
}

/// A unique mode-0700 directory containing one controller socket.
#[derive(Debug)]
pub struct FixtureSocketDirectory {
    path: PathBuf,
}

impl FixtureSocketDirectory {
    /// Creates a private short-path directory suitable for a Unix-domain socket.
    pub fn new() -> Result<Self, String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "the fixture socket clock was unavailable")?
            .as_nanos();
        for _attempt in 0..SOCKET_DIRECTORY_ATTEMPTS {
            let sequence = SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = PathBuf::from(format!(
                "/tmp/mado-pilot-fixture-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => {
                    let permissions = std::fs::Permissions::from_mode(0o700);
                    if std::fs::set_permissions(&path, permissions).is_err() {
                        let _removed = std::fs::remove_dir(&path);
                        return Err(
                            "the fixture socket directory could not be made private".to_owned()
                        );
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err("the fixture socket directory could not be created".to_owned());
                }
            }
        }
        Err("the fixture socket directory could not be made unique".to_owned())
    }

    /// Returns the socket path owned by this directory guard.
    pub fn socket_path(&self) -> PathBuf {
        self.path.join("control.sock")
    }
}

impl Drop for FixtureSocketDirectory {
    fn drop(&mut self) {
        let _socket_removed = std::fs::remove_file(self.socket_path());
        let _directory_removed = std::fs::remove_dir(&self.path);
    }
}

/// Authenticates one connected peer as the expected fixture executable.
pub fn authenticate_fixture_peer(
    stream: &UnixStream,
    expected_executable: &Path,
) -> Option<AuthenticatedFixtureProcess> {
    let expected_executable = std::fs::canonicalize(expected_executable).ok()?;
    let peer = fixture_peer_identity(stream)?;
    // SAFETY: `geteuid` has no arguments and returns process-local credentials.
    let current_effective_user_id = unsafe { geteuid() };
    fixture_peer_is_expected(&peer, current_effective_user_id, &expected_executable).then_some(
        AuthenticatedFixtureProcess {
            process_id: peer.process_id,
            audit_token: peer.audit_token,
        },
    )
}

fn fixture_peer_is_expected(
    peer: &FixturePeerIdentity,
    current_effective_user_id: u32,
    expected_executable: &Path,
) -> bool {
    peer.effective_user_id == current_effective_user_id
        && peer.process_id > 0
        && peer.executable == expected_executable
}

fn fixture_peer_identity(stream: &UnixStream) -> Option<FixturePeerIdentity> {
    let socket = stream.as_raw_fd();
    let mut effective_user_id = 0u32;
    let mut effective_group_id = 0u32;
    // SAFETY: both scalar outputs are writable and `socket` remains open for
    // the call. `getpeereid` reads credentials already bound to this connection.
    if unsafe {
        getpeereid(
            socket,
            &raw mut effective_user_id,
            &raw mut effective_group_id,
        )
    } != 0
    {
        return None;
    }

    let mut process_id = 0i32;
    let mut process_id_size = u32::try_from(size_of::<c_int>()).ok()?;
    // SAFETY: the output points to one writable `pid_t`, its exact byte extent
    // is supplied, and `LOCAL_PEERPID` reads the connected Unix peer only.
    if unsafe {
        getsockopt(
            socket,
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&raw mut process_id).cast::<c_void>(),
            &raw mut process_id_size,
        )
    } != 0
        || process_id_size as usize != size_of::<c_int>()
        || process_id <= 0
    {
        return None;
    }

    let mut audit_token = AuditToken { values: [0; 8] };
    let mut audit_token_size = u32::try_from(size_of::<AuditToken>()).ok()?;
    // SAFETY: the output is one writable audit token with its exact extent;
    // `LOCAL_PEERTOKEN` binds it to this connected peer and survives PID reuse.
    if unsafe {
        getsockopt(
            socket,
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            (&raw mut audit_token).cast::<c_void>(),
            &raw mut audit_token_size,
        )
    } != 0
        || audit_token_size as usize != size_of::<AuditToken>()
    {
        return None;
    }

    let mut executable = [0i8; PROC_PIDPATHINFO_MAXSIZE];
    // SAFETY: the fixed buffer is writable for the declared capacity and the
    // positive authenticated peer PID fits `proc_pidpath`'s `int` argument.
    let executable_len = unsafe {
        proc_pidpath(
            process_id,
            executable.as_mut_ptr().cast::<c_void>(),
            u32::try_from(executable.len()).ok()?,
        )
    };
    let Ok(executable_len) = usize::try_from(executable_len) else {
        return None;
    };
    if executable_len >= executable.len() {
        return None;
    }
    // SAFETY: the zero-initialized buffer retains a terminator beyond every
    // accepted result length.
    let executable = unsafe { CStr::from_ptr(executable.as_ptr()) };
    let executable = std::fs::canonicalize(Path::new(std::ffi::OsStr::from_bytes(
        executable.to_bytes(),
    )))
    .ok()?;
    Some(FixturePeerIdentity {
        effective_user_id,
        process_id: u32::try_from(process_id).ok()?,
        executable,
        audit_token,
    })
}

unsafe extern "C" {
    fn getpeereid(socket: c_int, effective_user: *mut u32, effective_group: *mut u32) -> c_int;
    fn geteuid() -> u32;
    fn getsockopt(
        socket: c_int,
        level: c_int,
        option: c_int,
        value: *mut c_void,
        value_size: *mut u32,
    ) -> c_int;
    fn mp_shim_input_environment(
        out_process: *mut i64,
        out_process_launch_time: *mut f64,
        out_pointer_x: *mut f64,
        out_pointer_y: *mut f64,
    ) -> u32;
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidpath(process_id: c_int, buffer: *mut c_void, buffer_size: u32) -> c_int;
    fn proc_pidpath_audittoken(
        audit_token: *mut AuditToken,
        buffer: *mut c_void,
        buffer_size: u32,
    ) -> c_int;
    fn proc_signal_with_audittoken(audit_token: *mut AuditToken, signal: c_int) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::{
        AuditToken, FixturePeerIdentity, FixtureSocketDirectory, authenticate_fixture_peer,
        fixture_peer_is_expected, next_fixture_run_nonce,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn concurrent_controllers_receive_distinct_strictly_ordered_run_identities() {
        const THREADS: usize = 16;
        const IDENTITIES_PER_THREAD: usize = 64;
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    (0..IDENTITIES_PER_THREAD)
                        .map(|_| next_fixture_run_nonce().expect("issue run identity"))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut identities = handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("nonce worker completes"))
            .collect::<Vec<_>>();
        identities.sort_unstable();

        assert_eq!(identities.len(), THREADS * IDENTITIES_PER_THREAD);
        assert!(identities.iter().all(|nonce| *nonce != 0));
        assert!(
            identities.windows(2).all(|pair| pair[0] < pair[1]),
            "no concurrent controller may reuse an earlier run identity"
        );
    }

    #[test]
    fn peer_identity_requires_the_expected_user_and_canonical_executable() {
        let expected_executable = Path::new("/private/tmp/approved-fixture");
        let peer = FixturePeerIdentity {
            effective_user_id: 501,
            process_id: 42,
            executable: expected_executable.to_path_buf(),
            audit_token: AuditToken { values: [7; 8] },
        };
        assert!(fixture_peer_is_expected(&peer, 501, expected_executable));

        let wrong_path = FixturePeerIdentity {
            executable: PathBuf::from("/private/tmp/lookalike-fixture"),
            ..peer.clone()
        };
        assert!(!fixture_peer_is_expected(
            &wrong_path,
            501,
            expected_executable
        ));
        assert!(!fixture_peer_is_expected(&peer, 502, expected_executable));
        assert!(!fixture_peer_is_expected(
            &FixturePeerIdentity {
                process_id: 0,
                ..peer
            },
            501,
            expected_executable
        ));
    }

    #[test]
    fn connected_peer_is_bound_to_the_current_executable_and_process() {
        let expected_executable = std::env::current_exe().expect("read current executable");
        let (server, _client) = UnixStream::pair().expect("create connected Unix sockets");
        let process = authenticate_fixture_peer(&server, &expected_executable)
            .expect("authenticate connected current process");
        assert_eq!(process.process_id(), std::process::id());
        assert!(process.matches_live_owner(i64::from(std::process::id())));
        assert!(!process.matches_live_owner(i64::from(std::process::id()) + 1));
    }

    #[test]
    fn socket_directory_is_unique_and_private() {
        let first = FixtureSocketDirectory::new().expect("create first private directory");
        let second = FixtureSocketDirectory::new().expect("create second private directory");
        assert_ne!(first.path, second.path);
        for directory in [&first, &second] {
            let mode = std::fs::metadata(&directory.path)
                .expect("the private directory exists")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
            assert_eq!(
                directory.socket_path().parent(),
                Some(directory.path.as_path())
            );
        }
    }
}
