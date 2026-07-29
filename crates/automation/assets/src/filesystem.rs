//! Stable filesystem opens for externally mutable package sources.
//!
//! Path metadata and a later `File::open` do not describe one object when the
//! path is replaced between those calls. This module obtains identity from open
//! handles, compares two opens around an operation checkpoint, and returns the
//! second handle for all later reads. A replacement before that retained open is
//! rejected; a replacement afterwards cannot redirect the retained handle.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use mado_pilot_core::Operation;

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Regular,
    Directory,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileState {
    identity: platform::FileIdentity,
    change: platform::ChangeStamp,
    len: u64,
    links: u64,
}

#[derive(Debug)]
pub(crate) struct OpenedNode {
    file: Option<File>,
    path: PathBuf,
    kind: NodeKind,
    state: FileState,
}

impl OpenedNode {
    pub(crate) const fn kind(&self) -> NodeKind {
        self.kind
    }

    pub(crate) const fn len(&self) -> u64 {
        self.state.len
    }

    pub(crate) const fn has_single_link(&self) -> bool {
        self.state.links == 1
    }

    pub(crate) fn changed(&self) -> bool {
        match self.file.as_ref() {
            Some(file) => match platform::state(file) {
                Ok((kind, state)) => kind != self.kind || state != self.state,
                Err(_) => true,
            },
            None => false,
        }
    }

    pub(crate) fn into_file(self) -> Option<OpenedFile> {
        self.file.map(|file| OpenedFile {
            file,
            state: self.state,
        })
    }
}

#[derive(Debug)]
pub(crate) struct OpenedFile {
    file: File,
    state: FileState,
}

impl OpenedFile {
    pub(crate) const fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    pub(crate) fn changed(&self) -> bool {
        match platform::state(&self.file) {
            Ok((_, state)) => state != self.state || state.links != 1,
            Err(_) => true,
        }
    }
}

pub(crate) fn open_stable(
    path: &Path,
    stage: LoadStage,
    operation: &mut Operation<'_>,
) -> Result<OpenedNode, AssetFault> {
    checkpoint(operation, stage)?;
    let first = platform::open_once(path).map_err(|error| open_fault(error, stage))?;

    // This is both an interruption boundary and the controlled race seam used by
    // conformance tests. Identity comes from the open handles on either side.
    checkpoint(operation, stage)?;
    let second = platform::open_once(path)
        .map_err(|_| AssetFault::new(AssetFaultKind::SourceChanged, stage))?;

    if first.kind != second.kind || first.state != second.state {
        return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
    }

    checkpoint(operation, stage)?;
    Ok(second)
}

pub(crate) fn open_child_stable(
    parent: &OpenedNode,
    name: &str,
    stage: LoadStage,
    operation: &mut Operation<'_>,
) -> Result<OpenedNode, AssetFault> {
    let directory = parent
        .file
        .as_ref()
        .ok_or_else(|| AssetFault::new(AssetFaultKind::SourceChanged, stage))?;
    checkpoint(operation, stage)?;
    let first = platform::open_child_once(directory, &parent.path, name)
        .map_err(|error| open_fault(error, stage))?;
    checkpoint(operation, stage)?;
    let second = platform::open_child_once(directory, &parent.path, name)
        .map_err(|_| AssetFault::new(AssetFaultKind::SourceChanged, stage))?;
    if first.kind != second.kind || first.state != second.state {
        return Err(AssetFault::new(AssetFaultKind::SourceChanged, stage));
    }
    checkpoint(operation, stage)?;
    Ok(second)
}

pub(crate) struct ChildEntries(platform::ChildEntries);

impl Iterator for ChildEntries {
    type Item = io::Result<OsString>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

pub(crate) fn read_children(
    parent: &OpenedNode,
    stage: LoadStage,
) -> Result<ChildEntries, AssetFault> {
    let directory = parent
        .file
        .as_ref()
        .ok_or_else(|| AssetFault::new(AssetFaultKind::SourceChanged, stage))?;
    platform::read_children(directory, &parent.path)
        .map(ChildEntries)
        .map_err(|_| AssetFault::new(AssetFaultKind::SourceUnreadable, stage))
}

fn checkpoint(operation: &mut Operation<'_>, stage: LoadStage) -> Result<(), AssetFault> {
    operation
        .checkpoint()
        .map_err(|interruption| AssetFault::interrupted(interruption, stage))
}

#[cfg(unix)]
const fn open_fault(error: platform::OpenError, stage: LoadStage) -> AssetFault {
    let kind = match error {
        platform::OpenError::Unreadable => AssetFaultKind::SourceUnreadable,
        platform::OpenError::Changed => AssetFaultKind::SourceChanged,
    };
    AssetFault::new(kind, stage)
}

#[cfg(windows)]
const fn open_fault(error: platform::OpenError, stage: LoadStage) -> AssetFault {
    match error {
        platform::OpenError::Unreadable => AssetFault::new(AssetFaultKind::SourceUnreadable, stage),
    }
}

#[cfg(unix)]
mod platform {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    use std::ffi::CStr;
    use std::ffi::{CString, OsString, c_char};
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::mem::offset_of;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    use super::{FileState, NodeKind, OpenedNode};

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NO_FOLLOW: i32 = 0x20_000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NONBLOCK: i32 = 0x800;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const CLOSE_ON_EXEC: i32 = 0x8_0000;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const LOOP_ERROR: i32 = 40;

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const NO_FOLLOW: i32 = 0x100;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const NONBLOCK: i32 = 0x4;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const CLOSE_ON_EXEC: i32 = 0x100_0000;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const LOOP_ERROR: i32 = 62;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct FileIdentity {
        device: u64,
        inode: u64,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct ChangeStamp {
        modified_seconds: i64,
        modified_nanoseconds: i64,
        changed_seconds: i64,
        changed_nanoseconds: i64,
    }

    #[derive(Debug, Clone, Copy)]
    pub(super) enum OpenError {
        Unreadable,
        Changed,
    }

    #[repr(C)]
    struct DirectoryStream {
        _private: [u8; 0],
    }

    /// The inline extent of `d_name`.
    ///
    /// `NAME_MAX + 1` on Linux, `__DARWIN_MAXPATHLEN` on the Apple platforms.
    /// Named because `entry_name` bounds the name against it, and a bound that
    /// restated the number could drift from the declaration it belongs to.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NAME_CAPACITY: usize = 256;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[repr(C)]
    struct DirectoryEntry {
        _inode: u64,
        _offset: i64,
        _record_len: u16,
        _file_type: u8,
        name: [c_char; NAME_CAPACITY],
    }

    /// The inline extent of `d_name`, which is `__DARWIN_MAXPATHLEN`.
    ///
    /// See the Linux definition above for why it is named.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const NAME_CAPACITY: usize = 1_024;

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[repr(C)]
    struct DirectoryEntry {
        _inode: u64,
        _seek_offset: u64,
        _record_len: u16,
        name_len: u16,
        _file_type: u8,
        name: [c_char; NAME_CAPACITY],
    }

    unsafe extern "C" {
        fn dup(file_descriptor: i32) -> i32;
        fn fdopendir(file_descriptor: i32) -> *mut DirectoryStream;
        fn readdir(directory: *mut DirectoryStream) -> *mut DirectoryEntry;
        fn closedir(directory: *mut DirectoryStream) -> i32;
        // Declared variadic because it is: `int openat(int, const char *, int, ...)`.
        // On `aarch64-apple-darwin` a variadic argument goes on the stack while a
        // fixed one goes in a register, so a fourth *fixed* parameter would put
        // the mode where the callee does not look for it. Nothing here passes a
        // creation flag, so no mode is passed either; a future `O_CREAT` must add
        // one as the variadic argument it is.
        fn openat(directory: i32, path: *const c_char, flags: i32, ...) -> i32;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        fn __errno_location() -> *mut i32;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        fn __error() -> *mut i32;
    }

    pub(super) fn open_once(path: &Path) -> Result<OpenedNode, OpenError> {
        let before = fs::symlink_metadata(path).map_err(|_| OpenError::Unreadable)?;
        let before_kind = kind(&before);
        let before_state = state_from_metadata(&before);

        if before_kind == NodeKind::Other {
            return Ok(OpenedNode {
                file: None,
                path: path.to_owned(),
                kind: before_kind,
                state: before_state,
            });
        }

        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NONBLOCK)
            .open(path)
            .map_err(|_| OpenError::Unreadable)?;
        let (after_kind, after_state) = state(&file).map_err(|_| OpenError::Unreadable)?;
        if before_kind != after_kind || before_state != after_state {
            return Err(OpenError::Changed);
        }

        Ok(OpenedNode {
            file: Some(file),
            path: path.to_owned(),
            kind: after_kind,
            state: after_state,
        })
    }

    pub(super) fn open_child_once(
        directory: &File,
        parent_path: &Path,
        name: &str,
    ) -> Result<OpenedNode, OpenError> {
        let child_path = parent_path.join(name);
        let child_name = CString::new(name.as_bytes()).map_err(|_| OpenError::Unreadable)?;
        // SAFETY: `directory` owns a live descriptor, `child_name` is
        // NUL-terminated, and the flags request a read-only, non-following child.
        let descriptor = unsafe {
            openat(
                directory.as_raw_fd(),
                child_name.as_ptr(),
                NO_FOLLOW | NONBLOCK | CLOSE_ON_EXEC,
            )
        };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(LOOP_ERROR) {
                return Ok(OpenedNode {
                    file: None,
                    path: child_path,
                    kind: NodeKind::Other,
                    state: FileState {
                        identity: FileIdentity {
                            device: 0,
                            inode: 0,
                        },
                        change: ChangeStamp {
                            modified_seconds: 0,
                            modified_nanoseconds: 0,
                            changed_seconds: 0,
                            changed_nanoseconds: 0,
                        },
                        len: 0,
                        links: 0,
                    },
                });
            }
            return Err(OpenError::Unreadable);
        }

        // SAFETY: `openat` returned a fresh owned descriptor.
        let file = unsafe { File::from_raw_fd(descriptor) };
        let (kind, state) = state(&file).map_err(|_| OpenError::Unreadable)?;
        Ok(OpenedNode {
            file: Some(file),
            path: child_path,
            kind,
            state,
        })
    }

    pub(super) struct ChildEntries {
        directory: *mut DirectoryStream,
    }

    impl Iterator for ChildEntries {
        type Item = io::Result<OsString>;

        fn next(&mut self) -> Option<Self::Item> {
            loop {
                // SAFETY: the platform errno accessor returns thread-local writable
                // state; clearing it distinguishes end-of-directory from failure.
                unsafe { *errno_location() = 0 };
                // SAFETY: `directory` remains owned by this iterator until Drop.
                let entry = unsafe { readdir(self.directory) };
                if entry.is_null() {
                    // SAFETY: this reads the same thread-local errno set by readdir.
                    let errno = unsafe { *errno_location() };
                    return (errno != 0).then(|| Err(io::Error::from_raw_os_error(errno)));
                }
                // SAFETY: `readdir` returned a live entry whose name is valid
                // until the next call on this same directory stream. The
                // pointer is passed on as a pointer: see `entry_name`.
                let name = unsafe { entry_name(entry) };
                if name.as_bytes() == b"." || name.as_bytes() == b".." {
                    continue;
                }
                return Some(Ok(name));
            }
        }
    }

    impl Drop for ChildEntries {
        fn drop(&mut self) {
            // SAFETY: `fdopendir` transferred one owned stream to this value.
            let _ = unsafe { closedir(self.directory) };
        }
    }

    pub(super) fn read_children(file: &File, _path: &Path) -> io::Result<ChildEntries> {
        // SAFETY: `file` owns a live descriptor; `dup` returns an independent
        // descriptor so `fdopendir` cannot consume the retained directory handle.
        let duplicate = unsafe { dup(file.as_raw_fd()) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `duplicate` is owned here and transferred on success.
        let directory = unsafe { fdopendir(duplicate) };
        if directory.is_null() {
            // SAFETY: `fdopendir` did not take ownership on failure.
            drop(unsafe { File::from_raw_fd(duplicate) });
            return Err(io::Error::last_os_error());
        }
        Ok(ChildEntries { directory })
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn errno_location() -> *mut i32 {
        // SAFETY: forwarded directly from the platform C runtime.
        unsafe { __errno_location() }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe fn errno_location() -> *mut i32 {
        // SAFETY: forwarded directly from the platform C runtime.
        unsafe { __error() }
    }

    /// Reads one directory entry's name.
    ///
    /// Takes a pointer rather than a reference, and that is the whole point.
    /// `readdir` returns a record `d_reclen` bytes long, and `d_reclen` covers
    /// only as much of `d_name` as the name needs — while `DirectoryEntry`
    /// declares `d_name` at its platform maximum, so the type is far larger than
    /// the record. Forming a `&DirectoryEntry` would assert that every byte of
    /// that larger type is valid and dereferenceable, which is undefined
    /// behaviour whether or not any byte past the record is read, and nothing
    /// here needs to read one. Projecting through the pointer with `&raw const`
    /// touches only the fields named.
    ///
    /// # Safety
    ///
    /// `entry` must be a non-null record `readdir` returned for a stream no
    /// later call has advanced, and must remain valid for this call.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn entry_name(entry: *const DirectoryEntry) -> OsString {
        // SAFETY: `d_reclen` sits at offset 16, inside every record `readdir`
        // writes, and the pointer libc returns is aligned for this struct.
        let record_len = usize::from(unsafe { (&raw const (*entry)._record_len).read() });
        let inside_record = record_len.saturating_sub(offset_of!(DirectoryEntry, name));
        let available = inside_record.min(NAME_CAPACITY);
        // SAFETY: `available` counts only bytes between `d_name` and the end of
        // the record, so the slice cannot leave what `readdir` wrote.
        let bytes = unsafe {
            std::slice::from_raw_parts((&raw const (*entry).name).cast::<u8>(), available)
        };
        // Bounded rather than scanned from a bare pointer. `readdir` does
        // NUL-terminate `d_name`, but relying on that for memory safety would
        // let a malformed record run the scan past the record; here it decides
        // only where the name ends inside bytes already proven to be ours.
        let name = CStr::from_bytes_until_nul(bytes).map_or(bytes, CStr::to_bytes);
        OsString::from_vec(name.to_vec())
    }

    /// Reads one directory entry's name.
    ///
    /// See the other `entry_name` for why this takes a pointer. This platform
    /// carries `d_namlen`, so the length is read rather than scanned for.
    ///
    /// # Safety
    ///
    /// As the other `entry_name`.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe fn entry_name(entry: *const DirectoryEntry) -> OsString {
        // SAFETY: `d_reclen` and `d_namlen` sit at offsets 16 and 18, inside
        // every record `readdir` writes. A plain read, not `read_unaligned`:
        // libc dereferences these fields itself, so the pointer it returns is
        // aligned for the struct. Measured on this host — 34 entries, none
        // misaligned.
        let record_len = usize::from(unsafe { (&raw const (*entry)._record_len).read() });
        // SAFETY: as above.
        let declared = usize::from(unsafe { (&raw const (*entry).name_len).read() });
        // Two bounds, and the record one is the load-bearing half. `d_namlen`
        // and the type's inline extent are both larger than the record: measured
        // here, `d_reclen` runs 32 to 56 bytes against a 1048-byte type, so a
        // `d_namlen` that disagreed with its own record would put the slice past
        // the bytes `readdir` wrote while still looking plausible.
        let inside_record = record_len.saturating_sub(offset_of!(DirectoryEntry, name));
        let length = declared.min(inside_record).min(NAME_CAPACITY);
        // SAFETY: `length` counts only bytes between `d_name` and the end of the
        // record, so the slice cannot leave what `readdir` wrote.
        let bytes =
            unsafe { std::slice::from_raw_parts((&raw const (*entry).name).cast::<u8>(), length) };
        OsString::from_vec(bytes.to_vec())
    }

    pub(super) fn state(file: &File) -> io::Result<(NodeKind, FileState)> {
        let metadata = file.metadata()?;
        Ok((kind(&metadata), state_from_metadata(&metadata)))
    }

    fn kind(metadata: &fs::Metadata) -> NodeKind {
        let file_type = metadata.file_type();
        if file_type.is_file() {
            NodeKind::Regular
        } else if file_type.is_dir() {
            NodeKind::Directory
        } else {
            NodeKind::Other
        }
    }

    fn state_from_metadata(metadata: &fs::Metadata) -> FileState {
        FileState {
            identity: FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            change: ChangeStamp {
                modified_seconds: metadata.mtime(),
                modified_nanoseconds: metadata.mtime_nsec(),
                changed_seconds: metadata.ctime(),
                changed_nanoseconds: metadata.ctime_nsec(),
            },
            len: metadata.len(),
            links: metadata.nlink(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{OsString, c_void};
    use std::fs::{self, File};
    use std::io;
    use std::mem::{MaybeUninit, size_of};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;
    use std::ptr;

    use super::{FileState, NodeKind, OpenedNode};

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const INVALID_HANDLE_VALUE: *mut c_void = usize::MAX as *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low_date_time: u32,
        high_date_time: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        file_attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[repr(C)]
    struct FileIdInfo {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }

    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }

    const FILE_STANDARD_INFO_CLASS: u32 = 1;
    const FILE_ID_INFO_CLASS: u32 = 18;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "CreateFileW"]
        fn create_file_w(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;

        #[link_name = "GetFileInformationByHandle"]
        fn get_file_information_by_handle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;

        #[link_name = "GetFileInformationByHandleEx"]
        fn get_file_information_by_handle_ex(
            file: *mut c_void,
            information_class: u32,
            information: *mut c_void,
            buffer_size: u32,
        ) -> i32;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct FileIdentity {
        volume: u64,
        id: [u8; 16],
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct ChangeStamp {
        creation: u64,
        last_write: u64,
    }

    #[derive(Debug, Clone, Copy)]
    pub(super) enum OpenError {
        Unreadable,
    }

    /// Opens one node by path, exactly as the caller spelled it.
    ///
    /// The path is passed through unchanged, so the Win32 limit of 260 applies
    /// and a source whose deepest path exceeds it reports `Unreadable`. Rust's
    /// `std` rewrites such a path into `\\?\` form; this does not, and the
    /// difference is deliberate rather than overlooked. Verbatim paths are not
    /// normalized by Win32 — `.` and `..` are passed to the filesystem instead
    /// of being resolved — and the containment rules this module enforces are
    /// written against the normalized form. Changing that is a change to link
    /// containment on Windows and belongs with the review that establishes it,
    /// not with a caller that wanted a longer path.
    ///
    /// What it costs is bounded: a package path is `templates/panel.png`, and a
    /// source that cannot be opened is a typed fault rather than a wrong answer.
    pub(super) fn open_once(path: &Path) -> Result<OpenedNode, OpenError> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(OpenError::Unreadable);
        }
        wide.push(0);

        // SAFETY: `wide` is NUL-terminated and lives for the call. Null optional
        // pointers are permitted, and a successful handle is immediately owned
        // by `File` below.
        let handle = unsafe {
            create_file_w(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(OpenError::Unreadable);
        }

        // SAFETY: `CreateFileW` returned a unique owned handle. `File` closes it
        // exactly once when this value or the retained node is dropped.
        let file = unsafe { File::from_raw_handle(handle) };
        let (kind, state) = state(&file).map_err(|_| OpenError::Unreadable)?;
        Ok(OpenedNode {
            file: Some(file),
            path: path.to_owned(),
            kind,
            state,
        })
    }

    pub(super) fn open_child_once(
        _directory: &File,
        parent_path: &Path,
        name: &str,
    ) -> Result<OpenedNode, OpenError> {
        open_once(&parent_path.join(name))
    }

    pub(super) struct ChildEntries(fs::ReadDir);

    impl Iterator for ChildEntries {
        type Item = io::Result<OsString>;

        fn next(&mut self) -> Option<Self::Item> {
            self.0
                .next()
                .map(|entry| entry.map(|entry| entry.file_name()))
        }
    }

    pub(super) fn read_children(_file: &File, path: &Path) -> io::Result<ChildEntries> {
        fs::read_dir(path).map(ChildEntries)
    }

    pub(super) fn state(file: &File) -> io::Result<(NodeKind, FileState)> {
        let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
        // SAFETY: `file` owns a valid handle and `information` points to writable
        // storage large enough for `BY_HANDLE_FILE_INFORMATION`.
        let succeeded = unsafe {
            get_file_information_by_handle(file.as_raw_handle(), information.as_mut_ptr())
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a nonzero result initializes the complete output structure.
        let information = unsafe { information.assume_init() };
        let identity = query_file_id(file)?;
        let standard = query_standard_info(file)?;
        if standard.delete_pending != 0 {
            return Err(io::Error::other("source entry is pending deletion"));
        }

        let kind = if information.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            NodeKind::Other
        } else if information.file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            NodeKind::Directory
        } else {
            NodeKind::Regular
        };
        Ok((
            kind,
            FileState {
                identity,
                change: ChangeStamp {
                    creation: file_time(information.creation_time),
                    last_write: file_time(information.last_write_time),
                },
                len: join_u32(information.file_size_high, information.file_size_low),
                links: u64::from(standard.number_of_links),
            },
        ))
    }

    fn query_file_id(file: &File) -> io::Result<FileIdentity> {
        let information = query_extended::<FileIdInfo>(file, FILE_ID_INFO_CLASS)?;
        Ok(FileIdentity {
            volume: information.volume_serial_number,
            id: information.file_id,
        })
    }

    fn query_standard_info(file: &File) -> io::Result<FileStandardInfo> {
        query_extended(file, FILE_STANDARD_INFO_CLASS)
    }

    fn query_extended<T>(file: &File, class: u32) -> io::Result<T> {
        let mut information = MaybeUninit::<T>::uninit();
        let size = u32::try_from(size_of::<T>())
            .map_err(|_| io::Error::other("file information structure is too large"))?;
        // SAFETY: `file` owns a valid handle, `information` is writable for
        // exactly `size` bytes, and the requested classes match `T` above.
        let succeeded = unsafe {
            get_file_information_by_handle_ex(
                file.as_raw_handle(),
                class,
                information.as_mut_ptr().cast(),
                size,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a nonzero result initializes the complete class-specific value.
        Ok(unsafe { information.assume_init() })
    }

    use std::os::windows::io::AsRawHandle;

    const fn join_u32(high: u32, low: u32) -> u64 {
        (high as u64) << 32 | low as u64
    }

    const fn file_time(time: FileTime) -> u64 {
        join_u32(time.high_date_time, time.low_date_time)
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use mado_pilot_core::{Operation, OperationContext};

    use super::{open_child_stable, open_stable, read_children};
    use crate::LoadStage;

    #[test]
    fn traversal_remains_bound_to_a_retained_root_after_path_replacement() {
        let parent = scratch("retained-root");
        let root = parent.join("source");
        let replacement = parent.join("replacement");
        let displaced = parent.join("displaced");
        fs::create_dir(&root).expect("source directory");
        fs::write(root.join("original.txt"), b"original").expect("original child");
        fs::create_dir(&replacement).expect("replacement directory");
        fs::write(replacement.join("replacement.txt"), b"replacement").expect("replacement child");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let opened = open_stable(&root, LoadStage::Source, &mut operation).expect("opened");

        fs::rename(&root, &displaced).expect("original root can move");
        fs::rename(&replacement, &root).expect("replacement takes the pathname");

        let names = read_children(&opened, LoadStage::Source)
            .expect("retained listing")
            .collect::<Result<Vec<_>, _>>()
            .expect("child names");
        assert_eq!(names, [std::ffi::OsString::from("original.txt")]);
        open_child_stable(&opened, "original.txt", LoadStage::Source, &mut operation)
            .expect("the child is opened relative to the retained root");
        assert!(
            open_child_stable(
                &opened,
                "replacement.txt",
                LoadStage::Source,
                &mut operation,
            )
            .is_err(),
            "the replacement tree is never traversed"
        );

        drop(opened);
        fs::remove_dir_all(parent).expect("cleanup");
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "mado-pilot-assets-unix-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("scratch directory");
        root
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::fs::{self, OpenOptions};
    use std::sync::atomic::{AtomicU64, Ordering};

    use mado_pilot_core::{Operation, OperationContext};

    use super::open_stable;
    use crate::LoadStage;

    #[test]
    fn a_retained_file_blocks_writers_and_path_replacement() {
        let root = scratch("file-sharing");
        let source = root.join("source.bin");
        let displaced = root.join("displaced.bin");
        fs::write(&source, b"stable").expect("source file");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let opened = open_stable(&source, LoadStage::Source, &mut operation).expect("opened");

        assert!(
            OpenOptions::new().write(true).open(&source).is_err(),
            "the retained source must deny concurrent writers"
        );
        assert!(
            fs::rename(&source, &displaced).is_err(),
            "the retained source must deny path replacement"
        );

        drop(opened);
        fs::rename(&source, &displaced).expect("replacement is possible after release");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_retained_directory_blocks_root_replacement() {
        let root = scratch("directory-sharing");
        let source = root.join("source");
        let displaced = root.join("displaced");
        fs::create_dir(&source).expect("source directory");
        let context = OperationContext::new();
        let mut operation = Operation::admit(&context).expect("admitted");
        let opened = open_stable(&source, LoadStage::Source, &mut operation).expect("opened");

        assert!(
            fs::rename(&source, &displaced).is_err(),
            "the retained root must deny path replacement"
        );

        drop(opened);
        fs::rename(&source, &displaced).expect("replacement is possible after release");
        fs::remove_dir_all(root).expect("cleanup");
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "mado-pilot-assets-windows-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("scratch directory");
        root
    }
}
