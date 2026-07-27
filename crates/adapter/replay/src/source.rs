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
use std::path::{Component, Path, PathBuf};

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

        let path = resolve_pixel_path(directory, &self.pixels)?;
        let pixels = fs::read(path).map_err(|_| ReplayFault::PixelsUnreadable)?;
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

/// Resolves a manifest-declared pixel path inside the source directory.
///
/// A replay manifest is caller-supplied data, so its paths are validated the way
/// an asset package's are: relative, no traversal, no root, no drive prefix, no
/// backslash, no path terminator. A replay source that could name
/// `../../etc/passwd` would be a file-read primitive wearing a capture adapter's
/// clothes.
fn resolve_pixel_path(directory: &Path, declared: &str) -> Result<PathBuf, ReplayFault> {
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

    let mut resolved = directory.to_path_buf();
    let mut segments = 0usize;
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
                resolved.push(other);
                segments += 1;
            }
        }
    }
    if segments == 0 {
        return Err(ReplayFault::UnsafePixelPath);
    }

    let metadata = fs::symlink_metadata(&resolved).map_err(|_| ReplayFault::PixelsUnreadable)?;
    if !metadata.is_file() {
        // `symlink_metadata` does not follow, so a symlink reports as a symlink
        // and is refused rather than followed out of the source directory.
        return Err(ReplayFault::UnsafePixelPath);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
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
        use super::resolve_pixel_path;
        use std::path::Path;

        let directory = Path::new("/nonexistent-replay-root");
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
                resolve_pixel_path(directory, declared).err(),
                Some(ReplayFault::UnsafePixelPath),
                "{declared}"
            );
        }
    }
}
