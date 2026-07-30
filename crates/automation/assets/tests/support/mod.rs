//! Shared fixtures and a minimal archive writer for the asset conformance tests.
//!
//! The writer stores entries uncompressed and is hand-rolled rather than taken
//! from the reader's own crate, so an archive built here cannot be wrong in the
//! same way the reader is. That is the same reason the tracked `G-014` fixtures
//! were produced by their own writer, and it is also what lets a test record an
//! entry count, a Unix mode, or a declared size that a conforming writer would
//! refuse to emit.

#![allow(dead_code, reason = "each integration test binary uses its own subset")]

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};

use mado_pilot_assets::{AssetPackage, ContentDigest, MemoryPackage, PackageLoader, PackageSource};
use mado_pilot_core::OperationContext;

/// Returns the root of the tracked `G-014` fixture set.
pub(crate) fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/assets/g-014")
}

/// Returns the tracked `tiny` directory package.
pub(crate) fn tiny_directory() -> PathBuf {
    fixture_root().join("valid/tiny-directory")
}

/// Returns the tracked `tiny` archive package.
pub(crate) fn tiny_archive() -> PathBuf {
    fixture_root().join("valid/valid-tiny.zip")
}

/// A directory that removes itself, so a test can mutate a package source
/// without touching the tracked fixtures.
///
/// Hand-rolled rather than taken from a crate: a temporary directory is thirty
/// lines here and a dependency review everywhere else.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates an empty directory named after `label`.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mado-pilot-assets-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a writable temporary directory");
        Self { path }
    }

    /// Returns the directory's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `bytes` to `relative`, creating any parent directories.
    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let target = self.path.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("a writable parent directory");
        }
        fs::write(&target, bytes).expect("a writable file");
        target
    }

    /// Copies the tracked `tiny` directory package into this directory.
    pub(crate) fn fill_with_tiny_package(&self) {
        copy_tree(&tiny_directory(), &self.path);
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a writable directory");
    for entry in fs::read_dir(from).expect("a readable fixture directory") {
        let entry = entry.expect("a readable directory entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("a copyable fixture");
        }
    }
}

/// Builds a memory package holding every file of the tracked `tiny` package.
pub(crate) fn tiny_memory_package() -> MemoryPackage {
    let root = tiny_directory();
    let mut package = MemoryPackage::new();
    for relative in tiny_relative_paths(&root, &root) {
        let bytes = fs::read(root.join(&relative)).expect("a readable fixture");
        package = package.with_entry(relative, bytes);
    }
    package
}

fn tiny_relative_paths(root: &Path, directory: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(directory).expect("a readable fixture directory") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            found.extend(tiny_relative_paths(root, &path));
            continue;
        }
        found.push(
            path.strip_prefix(root)
                .expect("below the root")
                .to_str()
                .expect("fixture paths are UTF-8")
                .replace('\\', "/"),
        );
    }
    found.sort();
    found
}

/// Returns the path of one tracked adversarial archive.
pub(crate) fn adversarial(name: &str) -> PathBuf {
    fixture_root().join("adversarial").join(name)
}

/// Loads a source under the implementation ceilings and no deadline.
pub(crate) fn load(source: &PackageSource) -> Result<AssetPackage, mado_pilot_assets::AssetFault> {
    PackageLoader::new().load(source, &OperationContext::new())
}

/// The manifest of the tracked `tiny` package, so a test can build an
/// equivalent memory or archive package from it.
pub(crate) fn tiny_manifest_bytes() -> Vec<u8> {
    fs::read(
        fixture_root()
            .join("valid/tiny-directory")
            .join(mado_pilot_assets::MANIFEST_PATH),
    )
    .expect("the tracked tiny manifest is readable")
}

/// A manifest that declares no templates, for tests about structure rather than
/// content.
pub(crate) fn empty_manifest() -> Vec<u8> {
    br#"{
      "schema_version": 1,
      "package": { "id": "madopilot.test.empty", "version": "1.0.0" },
      "license": "Apache-2.0",
      "templates": []
    }"#
    .to_vec()
}

/// Returns the lowercase hexadecimal SHA-256 of `bytes`.
pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    ContentDigest::of(bytes).to_string()
}

/// Builds a manifest declaring one template whose digest matches `content`.
pub(crate) fn single_template_manifest(path: &str, extent: (u32, u32), content: &[u8]) -> Vec<u8> {
    let digest = hex_sha256(content);
    let (width, height) = extent;
    format!(
        r#"{{
          "schema_version": 1,
          "package": {{ "id": "madopilot.test.single", "version": "1.0.0" }},
          "license": "Apache-2.0",
          "templates": [ {{
            "id": "only", "path": "{path}", "width": {width}, "height": {height},
            "coordinate_space": "capture_pixels",
            "content": {{ "algorithm": "sha256", "value": "{digest}" }},
            "match_defaults": {{ "min_score": 0.85, "max_results": 4 }}
          }} ]
        }}"#
    )
    .into_bytes()
}

/// The smallest byte string [`mado_pilot_vision::TemplateEncoding`] accepts.
pub(crate) const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// A clock that advances one millisecond every time it is read.
///
/// A deadline is then a count of context checks rather than a wall-clock wait,
/// so "the deadline expires while entries are being hashed" is an exact
/// statement about which check observes it instead of a race against a sleep.
#[derive(Debug)]
pub(crate) struct TickingClock {
    reads: AtomicU64,
}

impl TickingClock {
    /// Returns a clock at the domain origin.
    pub(crate) const fn new() -> Self {
        Self {
            reads: AtomicU64::new(0),
        }
    }

    /// Returns how many times the clock has been read.
    pub(crate) fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }
}

impl mado_pilot_core::Clock for TickingClock {
    fn now(&self) -> mado_pilot_core::MonotonicInstant {
        let tick = self.reads.fetch_add(1, Ordering::Relaxed);
        mado_pilot_core::MonotonicInstant::from_origin(std::time::Duration::from_millis(tick))
    }
}

/// A deterministic filesystem replacement staged through operation checkpoints.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct ReplacingClock {
    reads: AtomicU64,
    replace_at: u64,
    target: PathBuf,
    replacement: PathBuf,
    replaced: AtomicBool,
}

#[cfg(unix)]
impl ReplacingClock {
    pub(crate) fn new(
        replace_at: u64,
        target: impl Into<PathBuf>,
        replacement: impl Into<PathBuf>,
    ) -> Self {
        Self {
            reads: AtomicU64::new(0),
            replace_at,
            target: target.into(),
            replacement: replacement.into(),
            replaced: AtomicBool::new(false),
        }
    }

    pub(crate) fn replaced(&self) -> bool {
        self.replaced.load(Ordering::Relaxed)
    }
}

#[cfg(unix)]
impl mado_pilot_core::Clock for ReplacingClock {
    fn now(&self) -> mado_pilot_core::MonotonicInstant {
        let tick = self.reads.fetch_add(1, Ordering::Relaxed);
        if tick == self.replace_at {
            let displaced = self.target.with_file_name(format!(
                ".madopilot-displaced-{}",
                self.target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("source")
            ));
            if fs::rename(&self.target, &displaced).is_ok() {
                if fs::rename(&self.replacement, &self.target).is_ok() {
                    self.replaced.store(true, Ordering::Relaxed);
                } else {
                    let _ = fs::rename(&displaced, &self.target);
                }
            }
        }
        mado_pilot_core::MonotonicInstant::from_origin(std::time::Duration::from_millis(tick))
    }
}

/// A deterministic same-path rewrite staged through operation checkpoints.
#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct WritingClock {
    reads: AtomicU64,
    write_at: u64,
    target: PathBuf,
    bytes: Box<[u8]>,
    written: AtomicBool,
}

#[cfg(unix)]
impl WritingClock {
    pub(crate) fn new(write_at: u64, target: impl Into<PathBuf>, bytes: Box<[u8]>) -> Self {
        Self {
            reads: AtomicU64::new(0),
            write_at,
            target: target.into(),
            bytes,
            written: AtomicBool::new(false),
        }
    }

    pub(crate) fn written(&self) -> bool {
        self.written.load(Ordering::Relaxed)
    }
}

#[cfg(unix)]
impl mado_pilot_core::Clock for WritingClock {
    fn now(&self) -> mado_pilot_core::MonotonicInstant {
        let tick = self.reads.fetch_add(1, Ordering::Relaxed);
        if tick == self.write_at {
            fs::write(&self.target, &self.bytes).expect("the source remains writable on Unix");
            self.written.store(true, Ordering::Relaxed);
        }
        mado_pilot_core::MonotonicInstant::from_origin(std::time::Duration::from_millis(tick))
    }
}

/// A clock that cancels `token` once it has been read `after` times.
#[derive(Debug)]
pub(crate) struct CancellingClock {
    reads: AtomicU64,
    after: u64,
    token: mado_pilot_core::CancellationToken,
}

impl CancellingClock {
    /// Returns a clock that cancels `token` on its `after`-th read.
    pub(crate) const fn new(token: mado_pilot_core::CancellationToken, after: u64) -> Self {
        Self {
            reads: AtomicU64::new(0),
            after,
            token,
        }
    }
}

impl mado_pilot_core::Clock for CancellingClock {
    fn now(&self) -> mado_pilot_core::MonotonicInstant {
        let tick = self.reads.fetch_add(1, Ordering::Relaxed);
        if tick >= self.after {
            self.token.cancel();
        }
        mado_pilot_core::MonotonicInstant::from_origin(std::time::Duration::from_millis(tick))
    }
}

/// One entry to write into a test archive.
pub(crate) struct ArchiveEntry {
    pub name: Vec<u8>,
    pub content: Vec<u8>,
    /// The Unix mode recorded in the external attributes, file-type bits
    /// included.
    pub mode: u32,
}

impl ArchiveEntry {
    /// A regular file entry.
    pub(crate) fn file(name: &str, content: &[u8]) -> Self {
        Self {
            name: name.as_bytes().to_vec(),
            content: content.to_vec(),
            mode: 0o100_644,
        }
    }
}

/// Writes a ZIP archive whose entries are all stored uncompressed.
///
/// `recorded_entries` overrides the entry count written into the
/// end-of-central-directory record, so a test can build an archive that claims
/// more entries than it holds. `None` records the truth.
pub(crate) fn write_archive(entries: &[ArchiveEntry], recorded_entries: Option<u16>) -> Vec<u8> {
    const LOCAL_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
    const CENTRAL_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];

    let mut output: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();

    for entry in entries {
        let offset = u32::try_from(output.len()).expect("test archives stay small");
        let crc = crc32(&entry.content);
        let length = u32::try_from(entry.content.len()).expect("test entries stay small");
        let name_len = u16::try_from(entry.name.len()).expect("test names stay short");

        output.extend_from_slice(&LOCAL_SIGNATURE);
        output.extend_from_slice(&20u16.to_le_bytes()); // version needed
        output.extend_from_slice(&0u16.to_le_bytes()); // flags
        output.extend_from_slice(&0u16.to_le_bytes()); // stored
        output.extend_from_slice(&0u16.to_le_bytes()); // time
        output.extend_from_slice(&0u16.to_le_bytes()); // date
        output.extend_from_slice(&crc.to_le_bytes());
        output.extend_from_slice(&length.to_le_bytes()); // compressed
        output.extend_from_slice(&length.to_le_bytes()); // uncompressed
        output.extend_from_slice(&name_len.to_le_bytes());
        output.extend_from_slice(&0u16.to_le_bytes()); // extra length
        output.extend_from_slice(&entry.name);
        output.extend_from_slice(&entry.content);

        central.extend_from_slice(&CENTRAL_SIGNATURE);
        central.extend_from_slice(&0x031eu16.to_le_bytes()); // made by unix
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&length.to_le_bytes());
        central.extend_from_slice(&length.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&(entry.mode << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(&entry.name);
    }

    let central_offset = u32::try_from(output.len()).expect("test archives stay small");
    let central_size = u32::try_from(central.len()).expect("test archives stay small");
    let present = u16::try_from(entries.len()).expect("test archives stay small");
    let recorded = recorded_entries.unwrap_or(present);

    output.extend_from_slice(&central);
    output.extend_from_slice(&EOCD_SIGNATURE);
    output.extend_from_slice(&0u16.to_le_bytes()); // disk
    output.extend_from_slice(&0u16.to_le_bytes()); // directory disk
    output.extend_from_slice(&recorded.to_le_bytes());
    output.extend_from_slice(&recorded.to_le_bytes());
    output.extend_from_slice(&central_size.to_le_bytes());
    output.extend_from_slice(&central_offset.to_le_bytes());
    output.extend_from_slice(&0u16.to_le_bytes()); // comment length
    output
}

/// The CRC-32 a ZIP entry records, computed with the bit-reversed polynomial.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}
