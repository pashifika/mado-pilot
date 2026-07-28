//! The versioned manifest, and what a valid one has already proven.
//!
//! Parsing happens in two passes. The first reads only the schema version,
//! which is what lets a missing version, an unsupported version, and a
//! malformed document be three different answers instead of one parse error.
//! The second pass applies the typed schema for that version and rejects
//! unknown fields, so a manifest written for a later version fails on its
//! version rather than by having half of it silently ignored.
//!
//! A parsed [`Manifest`] has already validated everything that does not require
//! reading content: identities are unique, extents are non-zero, coordinate
//! metadata and matching defaults are values the vision contract accepts, paths
//! normalize safely, and every declared hash is a well-formed digest. What
//! remains is proving the bytes match, which is expansion's job.

use std::collections::BTreeSet;
use std::fmt;

use mado_pilot_core::{CoordinateSpace, PixelExtent};
use mado_pilot_vision::{MatchDefaults, TemplateId, VisionFault};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::path::PackagePath;

/// The package-relative path every manifest is read from.
pub const MANIFEST_PATH: &str = "madopilot-package.json";

/// The only manifest schema version this build implements.
pub const SCHEMA_VERSION: u32 = 1;

/// The only content hash algorithm this build implements.
pub const HASH_ALGORITHM: &str = "sha256";

/// A SHA-256 digest of one entry's exact bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentDigest([u8; 32]);

impl ContentDigest {
    /// Returns the digest of `content` under [`HASH_ALGORITHM`].
    ///
    /// A manifest must declare a digest for every entry and the loader verifies
    /// each one, so without this a caller assembling a package in memory had to
    /// add a hashing dependency of its own to state a value this package
    /// already computes. It is the same computation the loader performs, which
    /// is what makes a package built through it load rather than fail on its
    /// first hash.
    #[must_use]
    pub fn of(content: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(content);
        Self(<[u8; 32]>::from(hasher.finalize()))
    }

    /// Parses a 64-character hexadecimal digest, in either letter case.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 64 {
            return None;
        }
        let mut digest = [0u8; 32];
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            digest[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
        }
        Some(Self(digest))
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for ContentDigest {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Who produced a package, and what for.
///
/// Descriptive only. Nothing in loading or matching depends on it, and it is
/// optional because a package built by hand has nothing useful to put here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    created_by: String,
    created_for: Option<String>,
}

impl Provenance {
    /// Returns the tool or person that produced the package.
    #[must_use]
    pub fn created_by(&self) -> &str {
        &self.created_by
    }

    /// Returns what the package was produced for, if it says.
    #[must_use]
    pub fn created_for(&self) -> Option<&str> {
        self.created_for.as_deref()
    }
}

/// One validated template declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateDeclaration {
    id: TemplateId,
    path: PackagePath,
    extent: PixelExtent,
    space: CoordinateSpace,
    defaults: MatchDefaults,
    digest: ContentDigest,
}

impl TemplateDeclaration {
    /// Returns the template's identity.
    #[must_use]
    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    /// Returns the package path the content is read from.
    #[must_use]
    pub const fn path(&self) -> &PackagePath {
        &self.path
    }

    /// Returns the declared extent.
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

    /// Returns the digest the content must hash to.
    #[must_use]
    pub const fn digest(&self) -> ContentDigest {
        self.digest
    }
}

/// A parsed and validated package manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    package_id: String,
    package_version: String,
    license: String,
    provenance: Option<Provenance>,
    templates: Vec<TemplateDeclaration>,
}

impl Manifest {
    /// Parses and validates a manifest.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::MissingSchemaVersion`],
    /// [`AssetFaultKind::UnsupportedSchemaVersion`], or
    /// [`AssetFaultKind::MalformedManifest`] for a document this build cannot
    /// interpret, and a template-specific kind for a declaration whose values
    /// the contract does not accept. Every fault reports
    /// [`LoadStage::Manifest`].
    pub fn parse(bytes: &[u8]) -> Result<Self, AssetFault> {
        let probe: SchemaProbe = serde_json::from_slice(bytes).map_err(|_| malformed())?;
        let Some(declared) = probe.schema_version else {
            return Err(fault(AssetFaultKind::MissingSchemaVersion));
        };
        if declared != SCHEMA_VERSION {
            return Err(fault(AssetFaultKind::UnsupportedSchemaVersion));
        }

        let raw: RawManifest = serde_json::from_slice(bytes).map_err(|_| malformed())?;
        Self::validate(raw)
    }

    fn validate(raw: RawManifest) -> Result<Self, AssetFault> {
        if raw.package.id.is_empty() || raw.package.version.is_empty() || raw.license.is_empty() {
            return Err(malformed());
        }

        let mut identities = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut templates = Vec::with_capacity(raw.templates.len());
        for declaration in raw.templates {
            let declared = declaration.validate()?;
            if !identities.insert(declared.id.clone()) {
                return Err(fault(AssetFaultKind::DuplicateIdentity));
            }
            if !paths.insert(declared.path.clone()) {
                return Err(fault(AssetFaultKind::DuplicatePath));
            }
            templates.push(declared);
        }

        Ok(Self {
            package_id: raw.package.id,
            package_version: raw.package.version,
            license: raw.license,
            provenance: raw.provenance.map(|value| Provenance {
                created_by: value.created_by,
                created_for: value.created_for,
            }),
            templates,
        })
    }

    /// Returns the package identity.
    #[must_use]
    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    /// Returns the package version the producer declared.
    #[must_use]
    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    /// Returns the license the package content is offered under.
    #[must_use]
    pub fn license(&self) -> &str {
        &self.license
    }

    /// Returns the package's provenance, if it declares any.
    #[must_use]
    pub const fn provenance(&self) -> Option<&Provenance> {
        self.provenance.as_ref()
    }

    /// Returns every validated template declaration, in manifest order.
    #[must_use]
    pub fn templates(&self) -> &[TemplateDeclaration] {
        &self.templates
    }
}

#[derive(Deserialize)]
struct SchemaProbe {
    #[serde(default)]
    schema_version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    #[expect(
        dead_code,
        reason = "read by SchemaProbe; named here so deny_unknown_fields accepts it"
    )]
    schema_version: u32,
    package: RawPackage,
    license: String,
    #[serde(default)]
    provenance: Option<RawProvenance>,
    templates: Vec<RawTemplate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProvenance {
    created_by: String,
    #[serde(default)]
    created_for: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTemplate {
    id: String,
    path: String,
    width: u32,
    height: u32,
    coordinate_space: String,
    content: RawContent,
    match_defaults: RawMatchDefaults,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContent {
    algorithm: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMatchDefaults {
    min_score: f64,
    max_results: u32,
}

impl RawTemplate {
    fn validate(self) -> Result<TemplateDeclaration, AssetFault> {
        let id = TemplateId::new(self.id).map_err(from_vision)?;

        if self.path.contains("://") {
            return Err(fault(AssetFaultKind::UnsupportedSource));
        }
        let path = PackagePath::normalize(self.path.as_bytes()).ok_or_else(unsafe_manifest_path)?;

        let extent = PixelExtent::new(self.width, self.height);
        if extent.is_empty() {
            return Err(fault(AssetFaultKind::InvalidTemplateMetadata));
        }

        let space = coordinate_space(&self.coordinate_space)
            .ok_or_else(|| fault(AssetFaultKind::InvalidTemplateMetadata))?;
        if space != CoordinateSpace::CapturePixels {
            return Err(fault(AssetFaultKind::UnsupportedTemplateSpace));
        }

        if self.content.algorithm != HASH_ALGORITHM {
            return Err(fault(AssetFaultKind::UnsupportedHashAlgorithm));
        }
        let digest = ContentDigest::parse(&self.content.value)
            .ok_or_else(|| fault(AssetFaultKind::MalformedHash))?;

        let defaults = MatchDefaults::new(
            self.match_defaults.min_score,
            self.match_defaults.max_results,
        )
        .map_err(from_vision)?;

        Ok(TemplateDeclaration {
            id,
            path,
            extent,
            space,
            defaults,
            digest,
        })
    }
}

fn coordinate_space(slug: &str) -> Option<CoordinateSpace> {
    let spaces = [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ];
    spaces.into_iter().find(|space| space.as_str() == slug)
}

const fn fault(kind: AssetFaultKind) -> AssetFault {
    AssetFault::new(kind, LoadStage::Manifest)
}

const fn malformed() -> AssetFault {
    fault(AssetFaultKind::MalformedManifest)
}

const fn unsafe_manifest_path() -> AssetFault {
    fault(AssetFaultKind::UnsafePath)
}

/// Maps a vision rejection onto the asset axis, preserving the supported /
/// invalid distinction rather than flattening both into one category.
pub(crate) fn from_vision(fault_from_vision: VisionFault) -> AssetFault {
    let kind = match fault_from_vision {
        VisionFault::UnsupportedTemplateSpace => AssetFaultKind::UnsupportedTemplateSpace,
        VisionFault::EmptyTemplateContent => AssetFaultKind::UnsupportedContentEncoding,
        // A vision rule added later is metadata this manifest could not satisfy
        // until it is mapped deliberately, which is the safe default.
        _ => AssetFaultKind::InvalidTemplateMetadata,
    };
    fault(kind)
}

#[cfg(test)]
mod tests {
    use super::{ContentDigest, Manifest, SCHEMA_VERSION};
    use crate::fault::{AssetFaultKind, LoadStage};
    use mado_pilot_core::PixelExtent;

    const DIGEST: &str = "4d1b8f348988c4894a809f6a234361d6bf4019238b92112375652591edaaf7d4";

    fn manifest_json(template_fields: &str) -> String {
        format!(
            r#"{{
              "schema_version": {SCHEMA_VERSION},
              "package": {{ "id": "madopilot.fixture.tiny", "version": "1.0.0" }},
              "license": "Apache-2.0",
              "templates": [ {{ {template_fields} }} ]
            }}"#
        )
    }

    fn template_fields() -> String {
        format!(
            r#"
              "id": "template.0000",
              "path": "templates/0000-24x24.png",
              "width": 24,
              "height": 24,
              "coordinate_space": "capture_pixels",
              "content": {{ "algorithm": "sha256", "value": "{DIGEST}" }},
              "match_defaults": {{ "min_score": 0.9, "max_results": 8 }}
            "#
        )
    }

    fn parse_kind(json: &str) -> AssetFaultKind {
        let fault = Manifest::parse(json.as_bytes()).expect_err("rejected");
        assert_eq!(fault.stage(), LoadStage::Manifest);
        fault.kind()
    }

    #[test]
    fn a_valid_manifest_exposes_its_validated_declarations() {
        let manifest =
            Manifest::parse(manifest_json(&template_fields()).as_bytes()).expect("valid");

        assert_eq!(manifest.package_id(), "madopilot.fixture.tiny");
        assert_eq!(manifest.package_version(), "1.0.0");
        assert_eq!(manifest.license(), "Apache-2.0");
        assert_eq!(manifest.provenance(), None);

        let template = &manifest.templates()[0];
        assert_eq!(template.id().as_str(), "template.0000");
        assert_eq!(template.path().as_str(), "templates/0000-24x24.png");
        assert_eq!(template.extent(), PixelExtent::new(24, 24));
        assert_eq!(template.defaults().max_results(), 8);
        assert_eq!(template.digest().to_string(), DIGEST);
    }

    #[test]
    fn optional_provenance_is_read_when_present() {
        let json = format!(
            r#"{{
              "schema_version": 1,
              "package": {{ "id": "p", "version": "1" }},
              "license": "Apache-2.0",
              "provenance": {{ "created_by": "probe", "created_for": "evidence" }},
              "templates": [ {{ {} }} ]
            }}"#,
            template_fields()
        );
        let manifest = Manifest::parse(json.as_bytes()).expect("valid");
        let provenance = manifest.provenance().expect("declared");

        assert_eq!(provenance.created_by(), "probe");
        assert_eq!(provenance.created_for(), Some("evidence"));
    }

    #[test]
    fn a_missing_schema_version_is_not_the_same_answer_as_a_malformed_document() {
        let json =
            r#"{ "package": { "id": "p", "version": "1" }, "license": "x", "templates": [] }"#;

        assert_eq!(parse_kind(json), AssetFaultKind::MissingSchemaVersion);
    }

    #[test]
    fn an_unsupported_schema_version_is_reported_before_the_body_is_interpreted() {
        let json = r#"{ "schema_version": 2, "wildly": "wrong" }"#;

        assert_eq!(parse_kind(json), AssetFaultKind::UnsupportedSchemaVersion);
    }

    #[test]
    fn a_document_that_is_not_json_is_malformed() {
        assert_eq!(
            parse_kind("not json at all"),
            AssetFaultKind::MalformedManifest
        );
        assert_eq!(parse_kind(""), AssetFaultKind::MalformedManifest);
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        let json = manifest_json(&template_fields()).replace(
            r#""license": "Apache-2.0","#,
            r#""license": "Apache-2.0", "download_url": "https://example.invalid/p.zip","#,
        );

        assert_eq!(parse_kind(&json), AssetFaultKind::MalformedManifest);
    }

    #[test]
    fn a_remote_template_reference_is_refused_without_being_resolved() {
        let json = manifest_json(&template_fields().replace(
            r#""path": "templates/0000-24x24.png""#,
            r#""path": "https://example.invalid/0000.png""#,
        ));

        assert_eq!(parse_kind(&json), AssetFaultKind::UnsupportedSource);
    }

    #[test]
    fn an_unsafe_template_path_is_refused() {
        let json = manifest_json(&template_fields().replace(
            r#""path": "templates/0000-24x24.png""#,
            r#""path": "../outside.png""#,
        ));

        assert_eq!(parse_kind(&json), AssetFaultKind::UnsafePath);
    }

    #[test]
    fn duplicate_template_identities_are_refused() {
        let fields = template_fields();
        let json = format!(
            r#"{{
              "schema_version": 1,
              "package": {{ "id": "p", "version": "1" }},
              "license": "Apache-2.0",
              "templates": [ {{ {fields} }}, {{ {fields} }} ]
            }}"#
        );

        assert_eq!(parse_kind(&json), AssetFaultKind::DuplicateIdentity);
    }

    #[test]
    fn invalid_template_geometry_and_defaults_are_refused() {
        let zero_extent =
            manifest_json(&template_fields().replace(r#""width": 24"#, r#""width": 0"#));
        assert_eq!(
            parse_kind(&zero_extent),
            AssetFaultKind::InvalidTemplateMetadata
        );

        let bad_score =
            manifest_json(&template_fields().replace(r#""min_score": 0.9"#, r#""min_score": 1.4"#));
        assert_eq!(
            parse_kind(&bad_score),
            AssetFaultKind::InvalidTemplateMetadata
        );

        let zero_results =
            manifest_json(&template_fields().replace(r#""max_results": 8"#, r#""max_results": 0"#));
        assert_eq!(
            parse_kind(&zero_results),
            AssetFaultKind::InvalidTemplateMetadata
        );
    }

    #[test]
    fn a_template_outside_capture_pixels_is_unsupported_rather_than_malformed() {
        let json = manifest_json(&template_fields().replace(
            r#""coordinate_space": "capture_pixels""#,
            r#""coordinate_space": "frame_normalized""#,
        ));

        assert_eq!(parse_kind(&json), AssetFaultKind::UnsupportedTemplateSpace);

        let unknown = manifest_json(&template_fields().replace(
            r#""coordinate_space": "capture_pixels""#,
            r#""coordinate_space": "screen_inches""#,
        ));

        assert_eq!(
            parse_kind(&unknown),
            AssetFaultKind::InvalidTemplateMetadata
        );
    }

    #[test]
    fn hash_metadata_is_refused_by_algorithm_and_by_representation() {
        let algorithm = manifest_json(
            &template_fields().replace(r#""algorithm": "sha256""#, r#""algorithm": "md5""#),
        );
        assert_eq!(
            parse_kind(&algorithm),
            AssetFaultKind::UnsupportedHashAlgorithm
        );

        let short = manifest_json(&template_fields().replace(DIGEST, "abcdef"));
        assert_eq!(parse_kind(&short), AssetFaultKind::MalformedHash);

        let non_hex = manifest_json(&template_fields().replace(DIGEST, &"z".repeat(64)));
        assert_eq!(parse_kind(&non_hex), AssetFaultKind::MalformedHash);
    }

    #[test]
    fn a_digest_round_trips_through_either_letter_case() {
        let lower = ContentDigest::parse(DIGEST).expect("valid");
        let upper = ContentDigest::parse(&DIGEST.to_uppercase()).expect("valid");

        assert_eq!(lower, upper);
        assert_eq!(lower.to_string(), DIGEST);
    }
}
