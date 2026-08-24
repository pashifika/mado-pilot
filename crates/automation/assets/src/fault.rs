//! Why a package was refused, and how far the loader had got when it was.
//!
//! A refusal carries two independent facts. The [`AssetFaultKind`] says which
//! rule the package broke. The [`LoadStage`] says where the loader stopped, and
//! it is a contract in its own right: a package rejected later than its
//! documented stage means an earlier guard is missing, even though the package
//! was refused. `fixtures/assets/g-014/README.md` records the required stage for
//! every adversarial fixture, and the conformance tests assert both.
//!
//! Detail text is a fixed string per kind. It deliberately carries no entry
//! name, template identity, byte count, or file content: a diagnostic that
//! quotes an attacker-controlled name is a diagnostic that writes an
//! attacker-controlled name into a log.

use std::fmt;

use mado_pilot_core::{Error, Interruption, Status};

/// How far package loading had progressed when it stopped.
///
/// The archive stages are named after the ZIP structures they read, because
/// that is what makes "enforce before the cost" checkable: an entry count read
/// from the trailer costs a few dozen bytes, and the same count read after the
/// central directory is materialized costs megabytes.
///
/// Directory and memory sources have no trailer and no central directory, so
/// they enumerate at [`LoadStage::Source`] and share every later stage.
///
/// This enum is `#[non_exhaustive]`: later phases add stages, and a caller must
/// keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum LoadStage {
    /// Limits were rejected before any source was touched.
    Configuration,
    /// The source was measured and opened, and a directory or memory source
    /// enumerated its entries.
    Source,
    /// An archive's recorded entry count was read from its trailer, before the
    /// central directory was materialized.
    DirectoryPreParse,
    /// An archive's central directory was materialized and its declared total
    /// expansion was checked.
    DirectoryOpen,
    /// Entry names, types, declared sizes, and the aggregate declared ratio were
    /// checked, with nothing expanded.
    EntryMetadata,
    /// The manifest was read under its byte cap and parsed.
    Manifest,
    /// Referenced entries were streamed, size-checked, and hashed.
    Expansion,
    /// Every check had passed and the immutable package was being committed.
    Commit,
}

impl LoadStage {
    /// Returns a stable lowercase slug, for diagnostics and the C ABI mapping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LoadStage::Configuration => "configuration",
            LoadStage::Source => "source",
            LoadStage::DirectoryPreParse => "directory_pre_parse",
            LoadStage::DirectoryOpen => "directory_open",
            LoadStage::EntryMetadata => "entry_metadata",
            LoadStage::Manifest => "manifest",
            LoadStage::Expansion => "expansion",
            LoadStage::Commit => "commit",
        }
    }
}

impl fmt::Display for LoadStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which rule a package broke.
///
/// This enum is `#[non_exhaustive]`: later phases add rules, and a caller must
/// keep a fallback arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AssetFaultKind {
    /// A caller asked for a limit above the matching implementation ceiling.
    LimitAboveCeiling,
    /// The source could not be measured, opened, or read.
    SourceUnreadable,
    /// An externally mutable source changed while it was being read, so no
    /// consistent snapshot could be proven.
    SourceChanged,
    /// The archive's own structure is malformed or self-inconsistent.
    MalformedArchive,
    /// An archive entry uses a compression method the contract does not accept.
    UnsupportedCompressionMethod,
    /// An archive entry is encrypted.
    EncryptedEntry,
    /// An entry count, byte count, or expansion ratio would exceed its limit.
    ArchiveLimit,
    /// An entry name is absolute, rooted, traversing, or otherwise not a safe
    /// relative package path.
    UnsafePath,
    /// Two entries normalize to the same package path.
    DuplicatePath,
    /// An entry is a directory, link, device, or other non-regular type.
    UnsupportedEntryType,
    /// An entry produced a different number of bytes than it declared.
    DeclaredSizeMismatch,
    /// The package contains no manifest entry.
    MissingManifest,
    /// The manifest is not strict UTF-8 JSON matching the typed schema.
    MalformedManifest,
    /// The manifest omits its required schema version.
    MissingSchemaVersion,
    /// The manifest declares a schema version this build does not implement.
    UnsupportedSchemaVersion,
    /// Two manifest entries claim the same identity.
    DuplicateIdentity,
    /// The manifest references an entry the source does not contain.
    MissingEntry,
    /// A caller asked a committed package for a template it does not contain.
    UnknownTemplate,
    /// A caller asked a committed package for an OCR model it does not contain.
    UnknownOcrModel,
    /// The manifest requires content this loader will not fetch, such as a
    /// remote reference.
    UnsupportedSource,
    /// A template's declared extent, coordinate metadata, or matching defaults
    /// are not values the vision contract accepts.
    InvalidTemplateMetadata,
    /// An OCR model/profile declaration is incomplete, unbounded, or inconsistent.
    InvalidOcrModelMetadata,
    /// The package names OCR result-normalization semantics this build does not support.
    UnsupportedOcrProfile,
    /// A template declares its geometry in an unsupported coordinate space.
    UnsupportedTemplateSpace,
    /// The manifest declares a hash algorithm this build does not implement.
    UnsupportedHashAlgorithm,
    /// A declared hash value is not a well-formed digest.
    MalformedHash,
    /// An entry's computed hash differs from the one the manifest declared.
    HashMismatch,
    /// A template's content bytes are not an encoding this build accepts.
    UnsupportedContentEncoding,
    /// A size computation would have overflowed, which this loader is
    /// responsible for preventing.
    ArithmeticOverflow,
    /// The operation's cancellation token was set before the package committed.
    Cancelled,
    /// The operation's absolute deadline passed before the package committed.
    DeadlineExceeded,
}

impl AssetFaultKind {
    /// Returns the public status this kind reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            AssetFaultKind::LimitAboveCeiling
            | AssetFaultKind::UnknownTemplate
            | AssetFaultKind::UnknownOcrModel => Status::InvalidArgument,
            AssetFaultKind::ArchiveLimit => Status::LimitExceeded,
            AssetFaultKind::UnsupportedCompressionMethod
            | AssetFaultKind::EncryptedEntry
            | AssetFaultKind::UnsupportedSchemaVersion
            | AssetFaultKind::UnsupportedSource
            | AssetFaultKind::UnsupportedTemplateSpace
            | AssetFaultKind::UnsupportedHashAlgorithm
            | AssetFaultKind::UnsupportedContentEncoding
            | AssetFaultKind::UnsupportedOcrProfile => Status::Unsupported,
            AssetFaultKind::Cancelled => Status::Cancelled,
            AssetFaultKind::DeadlineExceeded => Status::DeadlineExceeded,
            AssetFaultKind::ArithmeticOverflow => Status::Internal,
            AssetFaultKind::SourceUnreadable
            | AssetFaultKind::SourceChanged
            | AssetFaultKind::MalformedArchive
            | AssetFaultKind::UnsafePath
            | AssetFaultKind::DuplicatePath
            | AssetFaultKind::UnsupportedEntryType
            | AssetFaultKind::DeclaredSizeMismatch
            | AssetFaultKind::MissingManifest
            | AssetFaultKind::MalformedManifest
            | AssetFaultKind::MissingSchemaVersion
            | AssetFaultKind::DuplicateIdentity
            | AssetFaultKind::MissingEntry
            | AssetFaultKind::InvalidTemplateMetadata
            | AssetFaultKind::InvalidOcrModelMetadata
            | AssetFaultKind::MalformedHash
            | AssetFaultKind::HashMismatch => Status::AssetInvalid,
        }
    }

    /// Returns a stable lowercase slug.
    ///
    /// These slugs are the failure categories tabulated in
    /// `fixtures/assets/g-014/README.md`, so the conformance table can be
    /// written from that document without a translation step.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AssetFaultKind::LimitAboveCeiling => "limit_above_ceiling",
            AssetFaultKind::SourceUnreadable => "source_unreadable",
            AssetFaultKind::SourceChanged => "source_changed",
            AssetFaultKind::MalformedArchive => "malformed_archive",
            AssetFaultKind::UnsupportedCompressionMethod => "unsupported_compression_method",
            AssetFaultKind::EncryptedEntry => "encrypted_entry",
            AssetFaultKind::ArchiveLimit => "archive_limit",
            AssetFaultKind::UnsafePath => "unsafe_path",
            AssetFaultKind::DuplicatePath => "duplicate_path",
            AssetFaultKind::UnsupportedEntryType => "unsupported_entry_type",
            AssetFaultKind::DeclaredSizeMismatch => "declared_size_mismatch",
            AssetFaultKind::MissingManifest => "missing_manifest",
            AssetFaultKind::MalformedManifest => "malformed_manifest",
            AssetFaultKind::MissingSchemaVersion => "missing_schema_version",
            AssetFaultKind::UnsupportedSchemaVersion => "unsupported_schema_version",
            AssetFaultKind::DuplicateIdentity => "duplicate_identity",
            AssetFaultKind::MissingEntry => "missing_entry",
            AssetFaultKind::UnknownTemplate => "unknown_template",
            AssetFaultKind::UnknownOcrModel => "unknown_ocr_model",
            AssetFaultKind::UnsupportedSource => "unsupported_source",
            AssetFaultKind::InvalidTemplateMetadata => "invalid_template_metadata",
            AssetFaultKind::InvalidOcrModelMetadata => "invalid_ocr_model_metadata",
            AssetFaultKind::UnsupportedOcrProfile => "unsupported_ocr_profile",
            AssetFaultKind::UnsupportedTemplateSpace => "unsupported_template_space",
            AssetFaultKind::UnsupportedHashAlgorithm => "unsupported_hash_algorithm",
            AssetFaultKind::MalformedHash => "malformed_hash",
            AssetFaultKind::HashMismatch => "hash_mismatch",
            AssetFaultKind::UnsupportedContentEncoding => "unsupported_content_encoding",
            AssetFaultKind::ArithmeticOverflow => "arithmetic_overflow",
            AssetFaultKind::Cancelled => "cancelled",
            AssetFaultKind::DeadlineExceeded => "deadline_exceeded",
        }
    }

    const fn detail(self) -> &'static str {
        match self {
            AssetFaultKind::LimitAboveCeiling => {
                "configured limit is above the implementation ceiling"
            }
            AssetFaultKind::SourceUnreadable => "package source could not be read",
            AssetFaultKind::SourceChanged => "package source changed while it was being read",
            AssetFaultKind::MalformedArchive => "archive structure is malformed",
            AssetFaultKind::UnsupportedCompressionMethod => {
                "archive entry uses an unsupported compression method"
            }
            AssetFaultKind::EncryptedEntry => "archive entry is encrypted",
            AssetFaultKind::ArchiveLimit => "package would exceed a configured archive limit",
            AssetFaultKind::UnsafePath => "entry name is not a safe relative package path",
            AssetFaultKind::DuplicatePath => "two entries normalize to the same package path",
            AssetFaultKind::UnsupportedEntryType => "entry is not a regular file",
            AssetFaultKind::DeclaredSizeMismatch => {
                "entry produced a different length than it declared"
            }
            AssetFaultKind::MissingManifest => "package contains no manifest",
            AssetFaultKind::MalformedManifest => "manifest is not valid for the typed schema",
            AssetFaultKind::MissingSchemaVersion => "manifest declares no schema version",
            AssetFaultKind::UnsupportedSchemaVersion => {
                "manifest declares an unsupported schema version"
            }
            AssetFaultKind::DuplicateIdentity => "two manifest entries claim the same identity",
            AssetFaultKind::MissingEntry => "manifest references an entry the package lacks",
            AssetFaultKind::UnknownTemplate => "package contains no template with that identity",
            AssetFaultKind::UnknownOcrModel => "package contains no OCR model with that identity",
            AssetFaultKind::UnsupportedSource => {
                "manifest requires content this loader will not fetch"
            }
            AssetFaultKind::InvalidTemplateMetadata => "template metadata is not a valid value",
            AssetFaultKind::InvalidOcrModelMetadata => {
                "OCR model metadata is incomplete, unbounded, or inconsistent"
            }
            AssetFaultKind::UnsupportedOcrProfile => "OCR profile normalization is not supported",
            AssetFaultKind::UnsupportedTemplateSpace => {
                "template geometry uses an unsupported coordinate space"
            }
            AssetFaultKind::UnsupportedHashAlgorithm => {
                "manifest declares an unsupported hash algorithm"
            }
            AssetFaultKind::MalformedHash => "declared hash value is malformed",
            AssetFaultKind::HashMismatch => "entry content does not match its declared hash",
            AssetFaultKind::UnsupportedContentEncoding => {
                "entry content is not a supported encoding"
            }
            AssetFaultKind::ArithmeticOverflow => "a size computation overflowed",
            AssetFaultKind::Cancelled => "operation was cancelled",
            AssetFaultKind::DeadlineExceeded => "operation deadline passed",
        }
    }
}

impl fmt::Display for AssetFaultKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A refused package: which rule it broke, and where the loader stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetFault {
    kind: AssetFaultKind,
    stage: LoadStage,
}

impl AssetFault {
    /// Builds a fault.
    #[must_use]
    pub const fn new(kind: AssetFaultKind, stage: LoadStage) -> Self {
        Self { kind, stage }
    }

    /// Returns the rule that was broken.
    #[must_use]
    pub const fn kind(self) -> AssetFaultKind {
        self.kind
    }

    /// Returns the stage the loader stopped at.
    #[must_use]
    pub const fn stage(self) -> LoadStage {
        self.stage
    }

    /// Returns the public status this fault reports as.
    #[must_use]
    pub const fn status(self) -> Status {
        self.kind.status()
    }

    pub(crate) const fn interrupted(interruption: Interruption, stage: LoadStage) -> Self {
        let kind = match interruption {
            Interruption::Cancelled => AssetFaultKind::Cancelled,
            Interruption::DeadlineExceeded => AssetFaultKind::DeadlineExceeded,
        };
        Self::new(kind, stage)
    }
}

impl fmt::Display for AssetFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.kind.detail(), self.stage)
    }
}

impl std::error::Error for AssetFault {}

impl From<AssetFault> for Error {
    fn from(fault: AssetFault) -> Self {
        Error::new(fault.status(), fault.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetFault, AssetFaultKind, LoadStage};
    use mado_pilot_core::{Error, Interruption, Status};

    #[test]
    fn kinds_map_to_public_statuses() {
        assert_eq!(
            AssetFaultKind::LimitAboveCeiling.status(),
            Status::InvalidArgument
        );
        assert_eq!(AssetFaultKind::ArchiveLimit.status(), Status::LimitExceeded);
        assert_eq!(AssetFaultKind::HashMismatch.status(), Status::AssetInvalid);
        assert_eq!(
            AssetFaultKind::UnsupportedSchemaVersion.status(),
            Status::Unsupported
        );
        assert_eq!(AssetFaultKind::Cancelled.status(), Status::Cancelled);
        assert_eq!(
            AssetFaultKind::DeadlineExceeded.status(),
            Status::DeadlineExceeded
        );
    }

    #[test]
    fn an_interruption_keeps_the_stage_it_was_observed_at() {
        let fault = AssetFault::interrupted(Interruption::Cancelled, LoadStage::Expansion);

        assert_eq!(fault.kind(), AssetFaultKind::Cancelled);
        assert_eq!(fault.stage(), LoadStage::Expansion);
        assert_eq!(fault.status(), Status::Cancelled);
    }

    #[test]
    fn a_fault_converts_into_the_public_error_naming_its_stage() {
        let error: Error =
            AssetFault::new(AssetFaultKind::UnsafePath, LoadStage::EntryMetadata).into();

        assert_eq!(error.status(), Status::AssetInvalid);
        assert!(error.detail().contains("entry_metadata"));
    }

    #[test]
    fn every_kind_slug_is_distinct() {
        let kinds = [
            AssetFaultKind::LimitAboveCeiling,
            AssetFaultKind::SourceUnreadable,
            AssetFaultKind::SourceChanged,
            AssetFaultKind::MalformedArchive,
            AssetFaultKind::UnsupportedCompressionMethod,
            AssetFaultKind::EncryptedEntry,
            AssetFaultKind::ArchiveLimit,
            AssetFaultKind::UnsafePath,
            AssetFaultKind::DuplicatePath,
            AssetFaultKind::UnsupportedEntryType,
            AssetFaultKind::DeclaredSizeMismatch,
            AssetFaultKind::MissingManifest,
            AssetFaultKind::MalformedManifest,
            AssetFaultKind::MissingSchemaVersion,
            AssetFaultKind::UnsupportedSchemaVersion,
            AssetFaultKind::DuplicateIdentity,
            AssetFaultKind::MissingEntry,
            AssetFaultKind::UnknownTemplate,
            AssetFaultKind::UnknownOcrModel,
            AssetFaultKind::UnsupportedSource,
            AssetFaultKind::InvalidTemplateMetadata,
            AssetFaultKind::InvalidOcrModelMetadata,
            AssetFaultKind::UnsupportedOcrProfile,
            AssetFaultKind::UnsupportedTemplateSpace,
            AssetFaultKind::UnsupportedHashAlgorithm,
            AssetFaultKind::MalformedHash,
            AssetFaultKind::HashMismatch,
            AssetFaultKind::UnsupportedContentEncoding,
            AssetFaultKind::ArithmeticOverflow,
            AssetFaultKind::Cancelled,
            AssetFaultKind::DeadlineExceeded,
        ];
        let mut slugs: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
        slugs.sort_unstable();
        let total = slugs.len();
        slugs.dedup();

        assert_eq!(slugs.len(), total);
    }

    #[test]
    fn stages_order_from_cheapest_to_most_expensive() {
        assert!(LoadStage::DirectoryPreParse < LoadStage::DirectoryOpen);
        assert!(LoadStage::DirectoryOpen < LoadStage::EntryMetadata);
        assert!(LoadStage::EntryMetadata < LoadStage::Manifest);
        assert!(LoadStage::Manifest < LoadStage::Expansion);
        assert!(LoadStage::Expansion < LoadStage::Commit);
    }
}
