//! What a template is, before any backend has compiled it.
//!
//! A template source is the neutral description a matching backend is given:
//! an identity, the encoded image bytes, the extent those bytes decode to, and
//! the defaults the template was authored with. It names no backend type, and
//! it names nothing about where the bytes came from — a caller may build one
//! from a file it read itself, and an asset package builds one from a validated
//! package entry. Neither arrangement is visible here, which is what keeps the
//! asset package optional.

use std::borrow::Borrow;
use std::fmt;
use std::sync::Arc;

use mado_pilot_core::{CoordinateSpace, PixelExtent};

use crate::fault::VisionFault;

/// A template's identity within the scope that produced it.
///
/// Identities are compared exactly. Nothing here folds case or normalizes
/// separators, because two identities that differ only in spelling are two
/// identities, and silently merging them would hide an authoring mistake.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TemplateId(Arc<str>);

impl TemplateId {
    /// Builds an identity.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::EmptyTemplateId`] for an empty identity.
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, VisionFault> {
        let value = value.into();
        if value.is_empty() {
            return Err(VisionFault::EmptyTemplateId);
        }
        Ok(Self(value))
    }

    /// Returns the identity as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TemplateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Lets a keyed collection be looked up by a plain string.
///
/// The borrowed form orders, compares, and hashes exactly as the identity does,
/// because both delegate to the same string.
impl Borrow<str> for TemplateId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// How a template's content bytes are encoded.
///
/// This enum is `#[non_exhaustive]`: later phases add encodings, and a caller
/// must keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TemplateEncoding {
    /// A PNG image, identified by its eight-byte signature.
    Png,
}

impl TemplateEncoding {
    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TemplateEncoding::Png => "png",
        }
    }

    /// Returns the encoding whose signature `content` begins with, if any.
    ///
    /// Content is identified by its own bytes rather than by a file extension
    /// or a declared field, because both of those are metadata a producer can
    /// get wrong and an attacker can choose.
    #[must_use]
    pub fn identify(content: &[u8]) -> Option<Self> {
        const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

        if content.starts_with(&PNG_SIGNATURE) {
            return Some(TemplateEncoding::Png);
        }
        None
    }
}

impl fmt::Display for TemplateEncoding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The matching options a template was authored with.
///
/// They are defaults, not a policy: a request may override either one. They
/// exist so a template that is known to need a stricter score travels with that
/// knowledge instead of relying on every caller to remember it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchDefaults {
    min_score: f64,
    max_results: u32,
}

impl MatchDefaults {
    /// Builds validated defaults.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::InvalidMatchScore`] when `min_score` is not a
    /// finite value inside `0.0..=1.0`, and
    /// [`VisionFault::InvalidMatchResultLimit`] when `max_results` is zero.
    pub fn new(min_score: f64, max_results: u32) -> Result<Self, VisionFault> {
        if !min_score.is_finite() || !(0.0..=1.0).contains(&min_score) {
            return Err(VisionFault::InvalidMatchScore);
        }
        if max_results == 0 {
            return Err(VisionFault::InvalidMatchResultLimit);
        }
        Ok(Self {
            min_score,
            max_results,
        })
    }

    /// Returns the lowest score a match must reach to qualify.
    #[must_use]
    pub const fn min_score(self) -> f64 {
        self.min_score
    }

    /// Returns the largest number of matches to report.
    #[must_use]
    pub const fn max_results(self) -> u32 {
        self.max_results
    }
}

/// The values [`TemplateSource::new`] validates.
///
/// A request object rather than six positional arguments, so a caller reading
/// the call site can tell the extent from the defaults without counting.
#[derive(Debug, Clone)]
pub struct TemplateSourceRequest {
    /// The template's identity.
    pub id: TemplateId,
    /// How `content` is encoded.
    pub encoding: TemplateEncoding,
    /// The extent `content` decodes to.
    pub extent: PixelExtent,
    /// The coordinate space `extent` is expressed in.
    pub space: CoordinateSpace,
    /// The matching options the template was authored with.
    pub defaults: MatchDefaults,
    /// The encoded image bytes.
    pub content: Arc<[u8]>,
}

/// An immutable template a backend can be asked to prepare.
///
/// Cloning shares the content bytes rather than copying them, so resolving the
/// same template twice costs an atomic increment. Nothing about the value can
/// change after construction, so a prepared template compiled from it stays
/// consistent with what the caller validated.
#[derive(Debug, Clone)]
pub struct TemplateSource {
    id: TemplateId,
    encoding: TemplateEncoding,
    extent: PixelExtent,
    space: CoordinateSpace,
    defaults: MatchDefaults,
    content: Arc<[u8]>,
}

impl TemplateSource {
    /// The largest number of pixels a template may declare: 67,108,864, which is
    /// 8,192 by 8,192.
    ///
    /// A declared extent is not a measurement, and this is the one thing about it
    /// this package can bound without decoding anything. A backend allocates its
    /// decoded image from the extent — three bytes a pixel for the Phase 1
    /// matching profile — so a compact image that declares 30,000 by 30,000 asks
    /// for 2.7 GB out of sixty-odd bytes of metadata, and asks for it before any
    /// content has been shown to support the claim.
    ///
    /// The number bounds that without refusing anything a template can be. A
    /// template is a patch of captured pixels, so its extent cannot usefully
    /// exceed the frame it is searched in, and 8,192 by 8,192 covers a full-frame
    /// template on any display through 8K UHD (7,680 by 4,320) with room over.
    /// What it admits at the top end is 192 MiB of three-channel pixels or 256
    /// MiB of four-channel ones — inside the 512 MiB of total expansion the asset
    /// ceilings already admit for one package, and orders of magnitude below the
    /// process-scale allocation an unbounded declaration reaches.
    ///
    /// Expressed in pixels rather than in bytes because bytes a pixel is a
    /// backend's decision: the product of two `u32` dimensions is exact in `u64`,
    /// and each backend multiplies it by the channel count it actually stores.
    /// Raising this ceiling is a decision for the phase that has a capture source
    /// or a matching profile needing it, with the measurements that phase takes.
    pub const MAX_PIXELS: u64 = 8_192 * 8_192;

    /// Returns whether `extent` declares at most [`TemplateSource::MAX_PIXELS`].
    ///
    /// Public so a producer of template metadata can apply the contract's own
    /// ceiling before it builds a source — an asset manifest refuses an oversized
    /// declaration while parsing, rather than after expanding the entry it names.
    #[must_use]
    pub const fn extent_within_ceiling(extent: PixelExtent) -> bool {
        // Two `u32` dimensions multiply exactly in `u64`, so there is nothing to
        // overflow and no saturation to reason about.
        extent.width() as u64 * extent.height() as u64 <= Self::MAX_PIXELS
    }

    /// Builds an immutable template source.
    ///
    /// The declared extent is not checked against the encoded image, because
    /// decoding is a backend's responsibility and doing it here would put an
    /// image parser in a contract package. A backend that decodes different
    /// dimensions reports that as its own failure. What is checked is the extent
    /// itself, against [`TemplateSource::MAX_PIXELS`]: that needs no decoder, and
    /// it is what keeps a backend from allocating from a declaration alone.
    ///
    /// # Errors
    ///
    /// Returns [`VisionFault::EmptyTemplateExtent`] for a zero dimension,
    /// [`VisionFault::TemplateExtentAboveCeiling`] for an extent above
    /// [`TemplateSource::MAX_PIXELS`], [`VisionFault::EmptyTemplateContent`] for
    /// empty content, and [`VisionFault::UnsupportedTemplateSpace`] for any
    /// coordinate space other than [`CoordinateSpace::CapturePixels`].
    pub fn new(request: TemplateSourceRequest) -> Result<Self, VisionFault> {
        let TemplateSourceRequest {
            id,
            encoding,
            extent,
            space,
            defaults,
            content,
        } = request;

        if extent.is_empty() {
            return Err(VisionFault::EmptyTemplateExtent);
        }
        if !Self::extent_within_ceiling(extent) {
            return Err(VisionFault::TemplateExtentAboveCeiling);
        }
        if content.is_empty() {
            return Err(VisionFault::EmptyTemplateContent);
        }
        if space != CoordinateSpace::CapturePixels {
            return Err(VisionFault::UnsupportedTemplateSpace);
        }

        Ok(Self {
            id,
            encoding,
            extent,
            space,
            defaults,
            content,
        })
    }

    /// Returns the template's identity.
    #[must_use]
    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    /// Returns how the content bytes are encoded.
    #[must_use]
    pub const fn encoding(&self) -> TemplateEncoding {
        self.encoding
    }

    /// Returns the extent the content decodes to.
    #[must_use]
    pub const fn extent(&self) -> PixelExtent {
        self.extent
    }

    /// Returns the coordinate space the extent is expressed in.
    #[must_use]
    pub const fn space(&self) -> CoordinateSpace {
        self.space
    }

    /// Returns the matching options the template was authored with.
    #[must_use]
    pub const fn defaults(&self) -> MatchDefaults {
        self.defaults
    }

    /// Returns the encoded image bytes.
    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }
}

impl PartialEq for TemplateSource {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.encoding == other.encoding
            && self.extent == other.extent
            && self.space == other.space
            && self.defaults == other.defaults
            && self.content == other.content
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mado_pilot_core::{CoordinateSpace, PixelExtent};

    use super::{
        MatchDefaults, TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest,
    };
    use crate::fault::VisionFault;

    const PNG: [u8; 9] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00];

    fn request() -> TemplateSourceRequest {
        TemplateSourceRequest {
            id: TemplateId::new("template.0000").expect("non-empty"),
            encoding: TemplateEncoding::Png,
            extent: PixelExtent::new(24, 24),
            space: CoordinateSpace::CapturePixels,
            defaults: MatchDefaults::new(0.9, 8).expect("valid"),
            content: Arc::from(PNG.as_slice()),
        }
    }

    #[test]
    fn an_empty_identity_is_rejected() {
        assert_eq!(TemplateId::new(""), Err(VisionFault::EmptyTemplateId));
    }

    #[test]
    fn encoding_is_identified_from_the_content_signature() {
        assert_eq!(
            TemplateEncoding::identify(&PNG),
            Some(TemplateEncoding::Png)
        );
        assert_eq!(TemplateEncoding::identify(b"GIF89a"), None);
        assert_eq!(TemplateEncoding::identify(b""), None);
    }

    #[test]
    fn match_defaults_reject_scores_outside_the_unit_range() {
        assert_eq!(
            MatchDefaults::new(1.5, 8),
            Err(VisionFault::InvalidMatchScore)
        );
        assert_eq!(
            MatchDefaults::new(f64::NAN, 8),
            Err(VisionFault::InvalidMatchScore)
        );
        assert_eq!(
            MatchDefaults::new(0.9, 0),
            Err(VisionFault::InvalidMatchResultLimit)
        );
        assert!(MatchDefaults::new(0.0, 1).is_ok());
        assert!(MatchDefaults::new(1.0, u32::MAX).is_ok());
    }

    #[test]
    fn a_valid_request_becomes_an_immutable_source() {
        let source = TemplateSource::new(request()).expect("valid");

        assert_eq!(source.id().as_str(), "template.0000");
        assert_eq!(source.extent(), PixelExtent::new(24, 24));
        assert_eq!(source.encoding(), TemplateEncoding::Png);
        assert_eq!(source.content(), PNG.as_slice());
        assert_eq!(source.defaults().min_score(), 0.9);
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        let mut invalid = request();
        invalid.extent = PixelExtent::new(24, 0);

        assert_eq!(
            TemplateSource::new(invalid),
            Err(VisionFault::EmptyTemplateExtent)
        );
    }

    #[test]
    fn an_extent_above_the_pixel_ceiling_is_rejected_without_decoding_anything() {
        // 30,000 by 30,000 is the shape of the declaration this ceiling exists
        // for: 2.7 GB of three-channel pixels, out of content a package can carry
        // in sixty bytes. Nothing here reads `content`, which is the point — the
        // refusal costs one multiplication.
        let mut invalid = request();
        invalid.extent = PixelExtent::new(30_000, 30_000);

        assert_eq!(
            TemplateSource::new(invalid),
            Err(VisionFault::TemplateExtentAboveCeiling)
        );
    }

    #[test]
    fn the_pixel_ceiling_admits_a_full_frame_template_and_refuses_one_pixel_more() {
        // The ceiling is a square, so a row of one pixel is the extent that
        // crosses it by exactly one pixel without changing its area class.
        assert!(TemplateSource::extent_within_ceiling(PixelExtent::new(
            8_192, 8_192
        )));
        assert!(!TemplateSource::extent_within_ceiling(PixelExtent::new(
            8_192, 8_193
        )));
        // An 8K UHD frame, which a template may cover entirely.
        assert!(TemplateSource::extent_within_ceiling(PixelExtent::new(
            7_680, 4_320
        )));
        // A single dimension at the top of its type is refused on area alone, so
        // no dimension can be large enough to matter to a backend's own extents.
        assert!(!TemplateSource::extent_within_ceiling(PixelExtent::new(
            u32::MAX,
            2
        )));
    }

    #[test]
    fn empty_content_is_rejected() {
        let mut invalid = request();
        invalid.content = Arc::from([].as_slice());

        assert_eq!(
            TemplateSource::new(invalid),
            Err(VisionFault::EmptyTemplateContent)
        );
    }

    #[test]
    fn a_template_declared_outside_capture_pixels_is_rejected() {
        let mut invalid = request();
        invalid.space = CoordinateSpace::FrameNormalized;

        assert_eq!(
            TemplateSource::new(invalid),
            Err(VisionFault::UnsupportedTemplateSpace)
        );
    }

    #[test]
    fn cloning_shares_the_content_rather_than_copying_it() {
        let source = TemplateSource::new(request()).expect("valid");
        let clone = source.clone();

        assert!(std::ptr::eq(source.content(), clone.content()));
    }
}
