//! What a replay source is, and how one is read from memory or from a directory.
//!
//! A replay source stores raw pixels rather than an encoded image. That is
//! deliberate: replay exists so a test, an example, and a benchmark all see
//! exactly the same bytes, and putting a codec between the fixture and the
//! oracle would make "the same" depend on a decoder's behavior instead of on the
//! fixture. It also keeps an image decoder out of a capture adapter, where it
//! would be an unreviewed parser reading caller-supplied files.
//!
//! The cost is real and worth stating: a raw fixture is large, so tracked replay
//! fixtures stay small, and a caller replaying real screenshots has to convert
//! them first.

use std::fs;
use std::io::Read;
use std::path::{Component, Path};

use mado_pilot_capture::{Continuity, FrameDescriptor, PixelFormat};
use mado_pilot_core::{Error, MonotonicInstant, PixelExtent, Scale, TargetPlacement};
use serde::Deserialize;

use crate::fault::ReplayFault;

/// The manifest file a directory replay source is described by.
pub const MANIFEST_NAME: &str = "madopilot-replay.json";

/// The only manifest schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

/// One frame a replay target will publish.
#[derive(Debug, Clone)]
pub struct ReplayFrame {
    descriptor: FrameDescriptor,
    placement: Option<TargetPlacement>,
    captured_at: MonotonicInstant,
    continuity: Continuity,
    pixels: Box<[u8]>,
}

impl ReplayFrame {
    /// Builds a replay frame from owned pixels.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayFault::FrameBytesMismatch`] when `pixels` is not exactly
    /// the length `descriptor` requires.
    pub fn new(
        descriptor: FrameDescriptor,
        captured_at: MonotonicInstant,
        continuity: Continuity,
        placement: Option<TargetPlacement>,
        pixels: Box<[u8]>,
    ) -> Result<Self, ReplayFault> {
        if pixels.len() != descriptor.byte_len() {
            return Err(ReplayFault::FrameBytesMismatch);
        }
        Ok(Self {
            descriptor,
            placement,
            captured_at,
            continuity,
            pixels,
        })
    }

    /// Returns the frame's extent, format, and stride.
    #[must_use]
    pub const fn descriptor(&self) -> FrameDescriptor {
        self.descriptor
    }

    /// Returns the declared target placement, if the source supplies one.
    #[must_use]
    pub const fn placement(&self) -> Option<TargetPlacement> {
        self.placement
    }

    /// Returns the frame's timestamp in the engine's monotonic domain.
    #[must_use]
    pub const fn captured_at(&self) -> MonotonicInstant {
        self.captured_at
    }

    /// Returns how this frame relates to the previous one.
    #[must_use]
    pub const fn continuity(&self) -> Continuity {
        self.continuity
    }

    /// Returns the frame's pixels.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub(crate) fn into_pixels(self) -> Box<[u8]> {
        self.pixels
    }
}

/// One replayable target and the sequence it publishes.
#[derive(Debug, Clone)]
pub struct ReplayTarget {
    name: String,
    frames: Vec<ReplayFrame>,
}

impl ReplayTarget {
    /// Builds a target from an ordered frame sequence.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayFault::EmptySequence`] for a target with no frames: a
    /// target that can never publish is not discoverable behavior, it is a
    /// configuration mistake, and reporting it at configuration time is cheaper
    /// than at the first frame request.
    pub fn new(name: impl Into<String>, frames: Vec<ReplayFrame>) -> Result<Self, ReplayFault> {
        if frames.is_empty() {
            return Err(ReplayFault::EmptySequence);
        }
        Ok(Self {
            name: name.into(),
            frames,
        })
    }

    /// Returns the descriptive target name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the frame sequence.
    #[must_use]
    pub fn frames(&self) -> &[ReplayFrame] {
        &self.frames
    }

    /// Returns the extent the target advertises, taken from its first frame.
    #[must_use]
    pub fn extent(&self) -> PixelExtent {
        self.frames[0].descriptor.extent()
    }

    /// Returns the pixel format the target advertises.
    #[must_use]
    pub fn format(&self) -> PixelFormat {
        self.frames[0].descriptor.format()
    }

    /// Reports whether every frame declares a target placement.
    ///
    /// Coordinate support is advertised for the whole target, so a sequence that
    /// declares placement for only some of its frames advertises none. A caller
    /// that was told a conversion works must not find it failing two frames
    /// later.
    #[must_use]
    pub fn declares_placement(&self) -> bool {
        self.frames.iter().all(|frame| frame.placement.is_some())
    }

    pub(crate) fn into_frames(self) -> Vec<ReplayFrame> {
        self.frames
    }
}

/// A configured set of replay targets.
///
/// Reading a source touches only what the caller pointed at. It never enumerates
/// the desktop, never asks for a screen-capture permission, and never opens a
/// network connection.
#[derive(Debug, Clone, Default)]
pub struct ReplaySource {
    targets: Vec<ReplayTarget>,
}

impl ReplaySource {
    /// Builds a source from targets held in memory.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayFault::DuplicateTargetName`] when two targets share a
    /// name, because a name is how a caller tells discovered targets apart.
    pub fn from_targets(targets: Vec<ReplayTarget>) -> Result<Self, ReplayFault> {
        let mut names: Vec<&str> = targets.iter().map(ReplayTarget::name).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        if names.len() != total {
            return Err(ReplayFault::DuplicateTargetName);
        }
        Ok(Self { targets })
    }

    /// Reads a source from a directory containing [`MANIFEST_NAME`].
    ///
    /// # Errors
    ///
    /// Returns a replay fault for a missing, malformed, or unsupported manifest,
    /// for an unsafe pixel path, and for a pixel file whose length disagrees with
    /// its declared descriptor.
    pub fn from_directory(directory: impl AsRef<Path>) -> Result<Self, ReplayFault> {
        let directory = directory.as_ref();
        let manifest_path = directory.join(MANIFEST_NAME);
        let text =
            fs::read_to_string(&manifest_path).map_err(|_| ReplayFault::ManifestUnreadable)?;
        let manifest: Manifest =
            serde_json::from_str(&text).map_err(|_| ReplayFault::ManifestMalformed)?;
        if manifest.schema_version != SCHEMA_VERSION {
            return Err(ReplayFault::UnsupportedSchemaVersion);
        }

        let mut targets = Vec::with_capacity(manifest.targets.len());
        for target in manifest.targets {
            let mut frames = Vec::with_capacity(target.frames.len());
            for frame in target.frames {
                frames.push(frame.load(directory)?);
            }
            targets.push(ReplayTarget::new(target.name, frames)?);
        }
        Self::from_targets(targets)
    }

    /// Returns the configured targets, in declaration order.
    #[must_use]
    pub fn targets(&self) -> &[ReplayTarget] {
        &self.targets
    }

    pub(crate) fn into_targets(self) -> Vec<ReplayTarget> {
        self.targets
    }
}

impl From<ReplayFault> for Error {
    fn from(fault: ReplayFault) -> Self {
        Error::new(fault.status(), fault.detail())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    targets: Vec<ManifestTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestTarget {
    name: String,
    frames: Vec<ManifestFrame>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFrame {
    pixels: String,
    width: u32,
    height: u32,
    format: String,
    #[serde(default)]
    stride: Option<usize>,
    #[serde(default)]
    captured_at_nanos: u64,
    #[serde(default = "default_continuity")]
    continuity: String,
    #[serde(default)]
    placement: Option<ManifestPlacement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPlacement {
    desktop_origin: [f64; 2],
    logical_size: [f64; 2],
    scale: [f64; 2],
}

fn default_continuity() -> String {
    "continuous".to_owned()
}

impl ManifestFrame {
    fn load(self, directory: &Path) -> Result<ReplayFrame, ReplayFault> {
        let extent = PixelExtent::new(self.width, self.height);
        let format = parse_format(&self.format)?;
        let descriptor = match self.stride {
            Some(stride) => FrameDescriptor::new(extent, format, stride),
            None => FrameDescriptor::packed(extent, format),
        }
        .map_err(|_| ReplayFault::FrameDescriptorInvalid)?;

        let pixels = read_pixel_file(directory, &self.pixels)?;
        let placement = self.placement.map(TryInto::try_into).transpose()?;

        ReplayFrame::new(
            descriptor,
            MonotonicInstant::from_origin(std::time::Duration::from_nanos(self.captured_at_nanos)),
            parse_continuity(&self.continuity)?,
            placement,
            pixels.into_boxed_slice(),
        )
    }
}

impl TryFrom<ManifestPlacement> for TargetPlacement {
    type Error = ReplayFault;

    fn try_from(placement: ManifestPlacement) -> Result<Self, Self::Error> {
        let scale = Scale::new(placement.scale[0], placement.scale[1])
            .map_err(|_| ReplayFault::PlacementInvalid)?;
        TargetPlacement::new(
            (placement.desktop_origin[0], placement.desktop_origin[1]),
            (placement.logical_size[0], placement.logical_size[1]),
            scale,
        )
        .map_err(|_| ReplayFault::PlacementInvalid)
    }
}

fn parse_format(name: &str) -> Result<PixelFormat, ReplayFault> {
    match name {
        "rgba8" => Ok(PixelFormat::Rgba8),
        "bgra8" => Ok(PixelFormat::Bgra8),
        _ => Err(ReplayFault::UnsupportedFormatName),
    }
}

fn parse_continuity(name: &str) -> Result<Continuity, ReplayFault> {
    match name {
        "continuous" => Ok(Continuity::Continuous),
        "geometry_changed" => Ok(Continuity::GeometryChanged),
        "discontinuous" => Ok(Continuity::Discontinuous),
        _ => Err(ReplayFault::UnsupportedContinuityName),
    }
}

/// Splits a manifest-declared pixel path into the components it may name.
///
/// A replay manifest is caller-supplied data, so its paths are validated the way
/// an asset package's are: relative, no traversal, no root, no drive prefix, no
/// backslash, no path terminator. A replay source that could name
/// `../../etc/passwd` would be a file-read primitive wearing a capture adapter's
/// clothes.
///
/// Syntax is half the rule. The other half is [`read_pixel_file`], which opens
/// these components one at a time, because a path that is spelled inside the
/// source directory still leaves it when one of its components is a link.
fn pixel_path_segments(declared: &str) -> Result<Vec<&str>, ReplayFault> {
    if declared.is_empty()
        || declared.contains('\0')
        || declared.contains('\\')
        || declared.starts_with('/')
    {
        return Err(ReplayFault::UnsafePixelPath);
    }
    let bytes = declared.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(ReplayFault::UnsafePixelPath);
    }

    let mut segments = Vec::new();
    for segment in declared.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err(ReplayFault::UnsafePixelPath),
            other => {
                if Path::new(other)
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
                {
                    return Err(ReplayFault::UnsafePixelPath);
                }
                segments.push(other);
            }
        }
    }
    if segments.is_empty() {
        return Err(ReplayFault::UnsafePixelPath);
    }
    Ok(segments)
}

/// Reads a manifest-declared pixel file from inside the source directory.
///
/// The declared path is walked one component at a time from a handle on the
/// source directory, and no component may be a link. Refusing only the last one
/// is a different and much weaker rule: `frames/pixels.bin` names any file on
/// the host the moment `frames` is a link, so a check that resolves the
/// intermediate components describes an object the name did not stay inside.
///
/// The pixels are then read from the handle the walk validated rather than from
/// the path, so the object read is the object checked and a replacement made
/// after the check cannot redirect the read.
///
/// The source directory itself is the caller's own argument to
/// [`ReplaySource::from_directory`] and is opened as given, links included. The
/// rule governs what the manifest names, which is the part the caller did not
/// choose.
fn read_pixel_file(directory: &Path, declared: &str) -> Result<Vec<u8>, ReplayFault> {
    let segments = pixel_path_segments(declared)?;
    let mut file = contained::open_pixel_file(directory, &segments)?;
    let mut pixels = Vec::new();
    file.read_to_end(&mut pixels)
        .map_err(|_| ReplayFault::PixelsUnreadable)?;
    Ok(pixels)
}

/// Opening a manifest-declared path without leaving the source directory.
///
/// The two platforms reach the same outcome by different means, which is why
/// this is a platform module rather than one path expression: a Unix walk never
/// re-resolves a name at all, and a Windows walk pins every component it
/// checked so a later resolution of the same name cannot land anywhere else.
///
/// `mado-pilot-assets` solves the same problem for package sources in
/// `crates/automation/assets/src/filesystem.rs`, and this follows its approach
/// deliberately: `O_NOFOLLOW` with the platform's `ELOOP`, and
/// `FILE_FLAG_OPEN_REPARSE_POINT` with a reparse point classified as a refusal.
/// The two are separate because that module additionally carries open-handle
/// identity comparison for an externally mutable package, which a replay source
/// does not need, and because this package's only edges are `mado-pilot-core`
/// and `mado-pilot-capture`.
#[cfg(unix)]
mod contained {
    use std::ffi::{CString, c_char};
    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::path::Path;

    use crate::fault::ReplayFault;

    // Open flags and the link error are the platform's own numbers, declared
    // here because this package has no C-library dependency and this is the
    // only place that needs them. `O_RDONLY` is zero on both and is omitted.
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

    unsafe extern "C" {
        // Declared variadic because it is: `int openat(int, const char *, int, ...)`.
        // On `aarch64-apple-darwin` a variadic argument goes on the stack while a
        // fixed one goes in a register, so a fourth *fixed* parameter would put
        // the mode where the callee does not look for it. Nothing here passes a
        // creation flag, so no mode is passed either; a future `O_CREAT` must add
        // one as the variadic argument it is.
        fn openat(directory: i32, path: *const c_char, flags: i32, ...) -> i32;
    }

    pub(super) fn open_pixel_file(
        directory: &Path,
        segments: &[&str],
    ) -> Result<File, ReplayFault> {
        let (name, parents) = segments.split_last().ok_or(ReplayFault::UnsafePixelPath)?;

        // Each child is opened relative to the descriptor of the component
        // before it, so no name in the path is ever looked up a second time and
        // the previous handle can be released as soon as the next one exists.
        let mut current = File::open(directory).map_err(|_| ReplayFault::PixelsUnreadable)?;
        for parent in parents {
            let child = open_child(&current, parent)?;
            if !kind(&child)?.is_dir() {
                return Err(ReplayFault::UnsafePixelPath);
            }
            current = child;
        }

        let file = open_child(&current, name)?;
        if !kind(&file)?.is_file() {
            return Err(ReplayFault::UnsafePixelPath);
        }
        Ok(file)
    }

    fn open_child(directory: &File, name: &str) -> Result<File, ReplayFault> {
        let name = CString::new(name.as_bytes()).map_err(|_| ReplayFault::UnsafePixelPath)?;
        // SAFETY: `directory` owns a live descriptor, `name` is NUL-terminated
        // and outlives the call, and the flags request a read-only child that is
        // never followed and never blocks the open on a reader.
        let descriptor = unsafe {
            openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                NO_FOLLOW | NONBLOCK | CLOSE_ON_EXEC,
            )
        };
        if descriptor < 0 {
            // `O_NOFOLLOW` reports a link as `ELOOP`. That is the refusal this
            // rule exists for, and it is not the same answer as a file the
            // caller's own directory simply does not have.
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(LOOP_ERROR) {
                ReplayFault::UnsafePixelPath
            } else {
                ReplayFault::PixelsUnreadable
            });
        }
        // SAFETY: `openat` returned a fresh descriptor owned by this call.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }

    fn kind(file: &File) -> Result<std::fs::FileType, ReplayFault> {
        file.metadata()
            .map(|metadata| metadata.file_type())
            .map_err(|_| ReplayFault::PixelsUnreadable)
    }
}

#[cfg(windows)]
mod contained {
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    use crate::fault::ReplayFault;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    pub(super) fn open_pixel_file(
        directory: &Path,
        segments: &[&str],
    ) -> Result<File, ReplayFault> {
        let (name, parents) = segments.split_last().ok_or(ReplayFault::UnsafePixelPath)?;

        // Windows resolves a whole path on every open and offers no `openat`, so
        // containment comes from holding what has been checked: a handle opened
        // for read sharing alone denies the rename and the delete a replacement
        // needs, so each later open resolves the components this walk verified.
        // The handles are needed only until the pixel file itself is open, since
        // the read then goes through that handle and resolves nothing.
        let mut held = Vec::with_capacity(segments.len());
        held.push(open_source_directory(directory)?);

        let mut path = directory.to_path_buf();
        for parent in parents {
            path.push(parent);
            let child = open_directory_component(&path)?;
            let metadata = child
                .metadata()
                .map_err(|_| ReplayFault::PixelsUnreadable)?;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_dir()
            {
                return Err(ReplayFault::UnsafePixelPath);
            }
            held.push(child);
        }

        path.push(name);
        let file = open_pixel_component(&path)?;
        let metadata = file.metadata().map_err(|_| ReplayFault::PixelsUnreadable)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_file() {
            return Err(ReplayFault::UnsafePixelPath);
        }

        // Released only here: the walk is what the components had to stay pinned
        // for, and the caller reads the handle rather than the name.
        drop(held);
        Ok(file)
    }

    /// Opens the caller's own source directory, following it as given.
    ///
    /// Read sharing alone, because this is the first component the rest of the
    /// walk resolves through. Backup semantics are what allow a directory to be
    /// opened at all.
    fn open_source_directory(path: &Path) -> Result<File, ReplayFault> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|_| ReplayFault::PixelsUnreadable)
    }

    /// Opens one manifest-declared directory component as itself, and pins it.
    ///
    /// `FILE_FLAG_OPEN_REPARSE_POINT` returns a handle to a symbolic link,
    /// junction, or mount point rather than to whatever it points at, so the
    /// caller can see what the component is and refuse it.
    fn open_directory_component(path: &Path) -> Result<File, ReplayFault> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| ReplayFault::PixelsUnreadable)
    }

    /// Opens the pixel file itself, again as itself rather than as a link.
    ///
    /// This one keeps the sharing a plain read has, because nothing resolves
    /// through it: the containment the directory components need comes from
    /// denying their replacement, and refusing to read a fixture some other
    /// process happens to hold open would only cost a caller a working load.
    fn open_pixel_component(path: &Path) -> Result<File, ReplayFault> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| ReplayFault::PixelsUnreadable)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{ReplayFrame, ReplaySource, ReplayTarget, parse_continuity, parse_format};
    use crate::fault::ReplayFault;
    use mado_pilot_capture::{Continuity, FrameDescriptor, PixelFormat};
    use mado_pilot_core::{MonotonicInstant, PixelExtent};

    fn frame(width: u32, height: u32, fill: u8) -> ReplayFrame {
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(width, height), PixelFormat::Rgba8)
                .expect("valid");
        ReplayFrame::new(
            descriptor,
            MonotonicInstant::ORIGIN,
            Continuity::Continuous,
            None,
            vec![fill; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("valid")
    }

    #[test]
    fn pixel_bytes_must_match_the_declared_descriptor() {
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(4, 4), PixelFormat::Rgba8).expect("valid");

        assert_eq!(
            ReplayFrame::new(
                descriptor,
                MonotonicInstant::ORIGIN,
                Continuity::Continuous,
                None,
                vec![0; descriptor.byte_len() - 1].into_boxed_slice(),
            )
            .err(),
            Some(ReplayFault::FrameBytesMismatch)
        );
    }

    #[test]
    fn a_target_with_no_frames_is_a_configuration_mistake() {
        assert_eq!(
            ReplayTarget::new("empty", Vec::new()).err(),
            Some(ReplayFault::EmptySequence)
        );
    }

    #[test]
    fn duplicate_target_names_are_refused() {
        let first = ReplayTarget::new("same", vec![frame(4, 4, 1)]).expect("valid");
        let second = ReplayTarget::new("same", vec![frame(4, 4, 2)]).expect("valid");

        assert_eq!(
            ReplaySource::from_targets(vec![first, second]).err(),
            Some(ReplayFault::DuplicateTargetName)
        );
    }

    #[test]
    fn placement_is_advertised_only_when_every_frame_declares_it() {
        use mado_pilot_core::{Scale, TargetPlacement};

        let placement =
            TargetPlacement::new((0.0, 0.0), (4.0, 4.0), Scale::new(1.0, 1.0).expect("valid"))
                .expect("valid");
        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(4, 4), PixelFormat::Rgba8).expect("valid");
        let with_placement = ReplayFrame::new(
            descriptor,
            MonotonicInstant::ORIGIN,
            Continuity::Continuous,
            Some(placement),
            vec![0; descriptor.byte_len()].into_boxed_slice(),
        )
        .expect("valid");

        let all =
            ReplayTarget::new("all", vec![with_placement.clone(), with_placement]).expect("valid");
        let some = ReplayTarget::new("some", vec![frame(4, 4, 1)]).expect("valid");

        assert!(all.declares_placement());
        assert!(!some.declares_placement());
    }

    #[test]
    fn format_and_continuity_names_are_a_closed_set() {
        assert_eq!(parse_format("rgba8"), Ok(PixelFormat::Rgba8));
        assert_eq!(parse_format("bgra8"), Ok(PixelFormat::Bgra8));
        assert_eq!(
            parse_format("rgb8"),
            Err(ReplayFault::UnsupportedFormatName)
        );
        assert_eq!(parse_continuity("continuous"), Ok(Continuity::Continuous));
        assert_eq!(
            parse_continuity("discontinuous"),
            Ok(Continuity::Discontinuous)
        );
        assert_eq!(
            parse_continuity("maybe"),
            Err(ReplayFault::UnsupportedContinuityName)
        );
    }

    #[test]
    fn an_unsafe_pixel_path_is_refused_before_anything_is_read() {
        use super::pixel_path_segments;

        for declared in [
            "../outside.bin",
            "frames/../../outside.bin",
            "/etc/passwd",
            "C:/Windows/system32/config",
            "frames\\pixels.bin",
            "",
            "./",
        ] {
            assert_eq!(
                pixel_path_segments(declared).err(),
                Some(ReplayFault::UnsafePixelPath),
                "{declared}"
            );
        }
    }

    #[test]
    fn a_symlinked_directory_component_cannot_reach_a_file_outside_the_source() {
        let root = scratch("symlinked-directory");
        let source = root.join("source");
        let outside = root.join("outside");
        fs::create_dir(&source).expect("source directory");
        fs::create_dir(&outside).expect("outside directory");
        fs::write(outside.join("secret.bin"), [0xAA; PIXEL_BYTES]).expect("outside pixels");
        if link_directory(&outside, &source.join("frames")).is_err() {
            // The host will not create a symbolic link, so there is nothing here
            // to refuse. Windows needs Developer Mode or the create-link right.
            fs::remove_dir_all(&root).expect("cleanup");
            return;
        }
        // `2x2 rgba8` is exactly the length of the file outside the source, so
        // the descriptor check cannot be what refuses this: a manifest author
        // picks the extent and the stride as freely as the path.
        write_manifest(&source, "frames/secret.bin");

        // The escape this refuses, spelled out: resolving the declared path as a
        // path reaches the file outside the source, which is what a check of the
        // last component only and a read that re-resolves the whole path did.
        assert_eq!(
            fs::read(source.join("frames").join("secret.bin")).expect("the link resolves"),
            [0xAA; PIXEL_BYTES],
            "the case must reproduce the escape it asserts against"
        );
        assert_eq!(
            ReplaySource::from_directory(&source).err(),
            Some(ReplayFault::UnsafePixelPath),
            "a linked intermediate component is refused rather than followed"
        );

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_symlinked_pixel_file_cannot_reach_a_file_outside_the_source() {
        let root = scratch("symlinked-file");
        let source = root.join("source");
        let outside = root.join("outside.bin");
        fs::create_dir(&source).expect("source directory");
        fs::write(&outside, [0xAA; PIXEL_BYTES]).expect("outside pixels");
        if link_file(&outside, &source.join("pixels.bin")).is_err() {
            fs::remove_dir_all(&root).expect("cleanup");
            return;
        }
        write_manifest(&source, "pixels.bin");

        assert_eq!(
            ReplaySource::from_directory(&source).err(),
            Some(ReplayFault::UnsafePixelPath),
            "a linked final component is refused rather than followed"
        );

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_pixel_path_naming_a_directory_is_refused() {
        let root = scratch("directory-pixels");
        let source = root.join("source");
        fs::create_dir(&source).expect("source directory");
        fs::create_dir(source.join("frames")).expect("frames directory");
        write_manifest(&source, "frames");

        assert_eq!(
            ReplaySource::from_directory(&source).err(),
            Some(ReplayFault::UnsafePixelPath)
        );

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_pixel_file_the_source_does_not_have_reads_as_unreadable_not_unsafe() {
        let root = scratch("missing-pixels");
        let source = root.join("source");
        fs::create_dir(&source).expect("source directory");
        write_manifest(&source, "frames/pixels.bin");

        assert_eq!(
            ReplaySource::from_directory(&source).err(),
            Some(ReplayFault::PixelsUnreadable),
            "a path the source simply does not have is not the same answer as a refusal"
        );

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn a_pixel_file_inside_a_real_subdirectory_loads_its_own_bytes() {
        let root = scratch("contained-read");
        let source = root.join("source");
        fs::create_dir(&source).expect("source directory");
        fs::create_dir(source.join("frames")).expect("frames directory");
        fs::write(
            source.join("frames").join("pixels.bin"),
            [0x5A; PIXEL_BYTES],
        )
        .expect("source pixels");
        write_manifest(&source, "frames/pixels.bin");

        let loaded = ReplaySource::from_directory(&source).expect("the source loads");
        assert_eq!(
            loaded.targets()[0].frames()[0].pixels(),
            &[0x5A; PIXEL_BYTES][..]
        );

        fs::remove_dir_all(&root).expect("cleanup");
    }

    /// The byte length of the `2x2 rgba8` frame every filesystem test declares.
    const PIXEL_BYTES: usize = 2 * 2 * 4;

    fn write_manifest(directory: &Path, declared_pixels: &str) {
        let manifest = format!(
            "{{\"schema_version\":1,\"targets\":[{{\"name\":\"only\",\"frames\":\
             [{{\"pixels\":\"{declared_pixels}\",\"width\":2,\"height\":2,\
             \"format\":\"rgba8\"}}]}}]}}"
        );
        fs::write(directory.join(super::MANIFEST_NAME), manifest).expect("manifest");
    }

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "mado-pilot-replay-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("scratch directory");
        root
    }

    #[cfg(unix)]
    fn link_directory(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn link_directory(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(unix)]
    fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
