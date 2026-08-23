//! The one pipeline every package goes through.
//!
//! Sources differ in what they can record and what can go wrong while reading
//! them. They do not differ in what makes a package valid, so the stages below
//! run once, in one order, for all three of them. That order is the contract
//! recorded in [ADR 0001] and reproduced in `docs/evidence/g-014/probe.md`:
//! every ceiling is checked before the allocation or expansion it bounds, and
//! recorded metadata may reject but never authorise.
//!
//! Nothing is observable until the last step. A failure at any stage — a hash
//! that does not match, a source that moved, a deadline that passed — discards
//! everything read so far, so no entry from a refused attempt is ever trusted.
//!
//! [ADR 0001]: https://github.com/pashifika/mado-pilot/blob/main/docs/adr/0001-asset-archive-container-and-safety-ceilings.md

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use mado_pilot_core::{Operation, OperationContext};
use mado_pilot_ocr::{
    ModelComponentIdentity, ModelId, OcrFault, OcrModelSource, OcrModelSourceRequest,
};
use mado_pilot_vision::{TemplateEncoding, TemplateId, TemplateSource, TemplateSourceRequest};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::filesystem::{self, NodeKind};
use crate::limits::AssetLimits;
use crate::manifest::{
    ContentDigest, MANIFEST_PATH, Manifest, OcrComponentDeclaration, from_vision,
};
use crate::package::AssetPackage;
use crate::path::PackagePath;
use crate::reader::{EntryKind, EntryReader, EntryStorage, RawEntry};
use crate::source::PackageSource;
use crate::{archive, directory, memory};

/// One entry that survived metadata validation.
#[derive(Debug)]
struct CheckedEntry {
    index: usize,
    declared_size: u64,
}

type ExpandedPackage = (
    BTreeMap<TemplateId, TemplateSource>,
    BTreeMap<ModelId, OcrModelSource>,
);

/// Loads validated asset packages under one set of limits.
///
/// A loader is cheap, immutable, and reusable. It holds no cache: a refused
/// package leaves nothing behind, and a successful one is already owned by the
/// caller, so there is no partially trusted state for a second load to find.
#[derive(Debug, Clone, Copy, Default)]
pub struct PackageLoader {
    limits: AssetLimits,
}

impl PackageLoader {
    /// Returns a loader that applies the implementation ceilings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a loader that applies `limits`.
    ///
    /// [`AssetLimits`] can only be built at or below the ceilings, so there is
    /// no way to reach this with a limit that weakens one.
    #[must_use]
    pub const fn with_limits(limits: AssetLimits) -> Self {
        Self { limits }
    }

    /// Returns the limits this loader applies.
    #[must_use]
    pub const fn limits(&self) -> AssetLimits {
        self.limits
    }

    /// Loads and validates a package from `source`.
    ///
    /// `context` is checked before admission, between stages, on every chunk of
    /// every entry, and immediately before commit. A package is returned only
    /// when the commit wins that race.
    ///
    /// # Errors
    ///
    /// Returns an [`AssetFault`] carrying the rule that was broken and the
    /// stage that caught it.
    pub fn load(
        &self,
        source: &PackageSource,
        context: &OperationContext,
    ) -> Result<AssetPackage, AssetFault> {
        load(source, self.limits, context)
    }

    /// Loads and validates a package from an archive this call borrows.
    ///
    /// `context` is checked exactly as [`PackageLoader::load`] checks it, and the
    /// stages are the same stages: a borrowed archive is the archive-bytes source
    /// without the ownership, not a second loader.
    ///
    /// Nothing retains `bytes`. The pipeline reads it while this call runs, and a
    /// committed package holds each template's content in its own allocation, so
    /// the archive is not part of what a package owns. That is what lets a
    /// boundary holding a caller's view for the duration of one call load from it
    /// directly: the alternative is an owned copy as large as the source ceiling
    /// admits, and an allocation that large is one a host under memory pressure
    /// can fail to satisfy.
    ///
    /// [`PackageSource::archive_bytes`] remains the entry for a caller that owns
    /// its bytes and wants a source it can keep and load more than once.
    ///
    /// # Errors
    ///
    /// Returns an [`AssetFault`] carrying the rule that was broken and the stage
    /// that caught it, including the configured source ceiling at
    /// [`LoadStage::Source`].
    ///
    /// [`PackageSource::archive_bytes`]: crate::PackageSource::archive_bytes
    pub fn load_archive_bytes(
        &self,
        bytes: &[u8],
        context: &OperationContext,
    ) -> Result<AssetPackage, AssetFault> {
        load_archive_bytes(bytes, self.limits, context)
    }
}

fn load(
    source: &PackageSource,
    limits: AssetLimits,
    context: &OperationContext,
) -> Result<AssetPackage, AssetFault> {
    let mut operation = Operation::admit(context)
        .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Source))?;

    let (mut reader, raw) = open(source, limits, &mut operation)?;
    commit_package(
        reader.as_mut(),
        &raw,
        source.is_archive(),
        limits,
        operation,
    )
}

fn load_archive_bytes(
    bytes: &[u8],
    limits: AssetLimits,
    context: &OperationContext,
) -> Result<AssetPackage, AssetFault> {
    let mut operation = Operation::admit(context)
        .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Source))?;

    let length = u64::try_from(bytes.len())
        .map_err(|_| AssetFault::new(AssetFaultKind::ArithmeticOverflow, LoadStage::Source))?;
    // The same archive stages the owned kind runs, over a `Cursor` that borrows
    // rather than one that owns. The source ceiling is applied there, so a
    // borrowed archive is admitted by the same length rule as every other.
    let (mut reader, raw) =
        archive::open(Cursor::new(bytes), length, limits, &mut operation, None)?;
    commit_package(reader.as_mut(), &raw, true, limits, operation)
}

/// Runs every stage after the source is open, and commits.
///
/// One tail for both entries: what a source is changes how its bytes are reached
/// and nothing about what makes the package valid, so the ordered rules and the
/// final commit check live here rather than once per entry.
fn commit_package(
    reader: &mut dyn EntryReader,
    raw: &[RawEntry],
    is_archive: bool,
    limits: AssetLimits,
    mut operation: Operation<'_>,
) -> Result<AssetPackage, AssetFault> {
    checkpoint(&mut operation, LoadStage::Source)?;

    let table = validate_entries(raw, limits, is_archive)?;
    checkpoint(&mut operation, LoadStage::EntryMetadata)?;

    let manifest = read_manifest(reader, &table, &mut operation)?;
    checkpoint(&mut operation, LoadStage::Manifest)?;

    let (templates, ocr_models) = expand(reader, &table, &manifest, &mut operation)?;

    checkpoint(&mut operation, LoadStage::Commit)?;
    operation
        .commit(AssetPackage::new(manifest, templates, ocr_models))
        .map_err(|interruption| AssetFault::interrupted(interruption, LoadStage::Commit))
}

fn open(
    source: &PackageSource,
    limits: AssetLimits,
    operation: &mut Operation<'_>,
) -> Result<(Box<dyn EntryReader>, Vec<RawEntry>), AssetFault> {
    match source {
        PackageSource::Directory(root) => directory::open(root, limits, operation),
        PackageSource::Memory(package) => memory::open(package, limits),
        PackageSource::ArchiveFile(path) => {
            let opened = filesystem::open_stable(path, LoadStage::Source, operation)?;
            if opened.kind() != NodeKind::Regular || !opened.has_single_link() {
                return Err(AssetFault::new(
                    AssetFaultKind::SourceUnreadable,
                    LoadStage::Source,
                ));
            }
            let length = opened.len();
            let source = opened
                .into_file()
                .ok_or_else(|| AssetFault::new(AssetFaultKind::SourceChanged, LoadStage::Source))?;
            // The handle itself goes in rather than a reader over it: an
            // externally mutable file is copied once, under the source ceiling,
            // so every archive stage reads one unchanging sequence of bytes.
            archive::open_file(source, length, limits, operation)
        }
        PackageSource::ArchiveBytes(bytes) => {
            let length = u64::try_from(bytes.len()).map_err(|_| {
                AssetFault::new(AssetFaultKind::ArithmeticOverflow, LoadStage::Source)
            })?;
            archive::open(
                Cursor::new(Arc::clone(bytes)),
                length,
                limits,
                operation,
                None,
            )
        }
    }
}

/// Stage D. Applies the per-entry rules in the order [ADR 0001] fixes, then the
/// aggregate ratio.
///
/// The ratio is checked last of the metadata rules rather than first. Both
/// positions are before any expansion, so safety is identical, but a package
/// that crosses one absolute ceiling is far more likely to be a
/// misconfiguration than an attack, and naming that ceiling is the more
/// actionable diagnostic.
///
/// [ADR 0001]: https://github.com/pashifika/mado-pilot/blob/main/docs/adr/0001-asset-archive-container-and-safety-ceilings.md
fn validate_entries(
    raw: &[RawEntry],
    limits: AssetLimits,
    is_archive: bool,
) -> Result<BTreeMap<PackagePath, CheckedEntry>, AssetFault> {
    let mut table: BTreeMap<PackagePath, CheckedEntry> = BTreeMap::new();
    let mut total_uncompressed: u64 = 0;
    let mut total_compressed: u64 = 0;

    for (index, entry) in raw.iter().enumerate() {
        match entry.storage {
            EntryStorage::Accepted => {}
            EntryStorage::UnsupportedMethod => {
                return Err(fault(AssetFaultKind::UnsupportedCompressionMethod));
            }
            EntryStorage::Encrypted => return Err(fault(AssetFaultKind::EncryptedEntry)),
        }

        let path =
            PackagePath::normalize(&entry.name).ok_or_else(|| fault(AssetFaultKind::UnsafePath))?;

        if entry.kind != EntryKind::Regular {
            return Err(fault(AssetFaultKind::UnsupportedEntryType));
        }

        let ceiling = if path.as_str() == MANIFEST_PATH {
            limits.max_manifest_bytes()
        } else {
            limits.max_entry_uncompressed_bytes()
        };
        if entry.declared_size > ceiling {
            return Err(fault(AssetFaultKind::ArchiveLimit));
        }

        total_uncompressed = total_uncompressed
            .checked_add(entry.declared_size)
            .ok_or_else(|| fault(AssetFaultKind::ArithmeticOverflow))?;
        if total_uncompressed > limits.max_total_uncompressed_bytes() {
            return Err(fault(AssetFaultKind::ArchiveLimit));
        }
        total_compressed = total_compressed
            .checked_add(entry.compressed_size)
            .ok_or_else(|| fault(AssetFaultKind::ArithmeticOverflow))?;

        if table
            .insert(
                path,
                CheckedEntry {
                    index,
                    declared_size: entry.declared_size,
                },
            )
            .is_some()
        {
            return Err(fault(AssetFaultKind::DuplicatePath));
        }
    }

    if is_archive && exceeds_ratio(total_uncompressed, total_compressed, limits) {
        return Err(fault(AssetFaultKind::ArchiveLimit));
    }
    Ok(table)
}

/// Reports whether the declared expansion is more than the ratio limit allows.
///
/// Multiplication rather than division, so a zero compressed total is a
/// question about whether anything expands from nothing rather than a divide by
/// zero: an archive of empty entries expands to nothing and passes.
fn exceeds_ratio(uncompressed: u64, compressed: u64, limits: AssetLimits) -> bool {
    let allowed = compressed.saturating_mul(u64::from(limits.max_compression_ratio()));
    uncompressed > allowed
}

/// Stage E.
fn read_manifest(
    reader: &mut dyn EntryReader,
    table: &BTreeMap<PackagePath, CheckedEntry>,
    operation: &mut Operation<'_>,
) -> Result<Manifest, AssetFault> {
    let path = PackagePath::normalize(MANIFEST_PATH.as_bytes())
        .expect("the manifest path is a compile-time constant and normalizes");
    let entry = table
        .get(&path)
        .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingManifest, LoadStage::Manifest))?;

    let bytes = reader.read_entry(
        entry.index,
        entry.declared_size,
        LoadStage::Manifest,
        operation,
    )?;
    Manifest::parse(&bytes)
}

/// Stage F. Expands, size-checks, hashes, and identifies every referenced
/// entry, then builds the vision template each one resolves to.
fn expand(
    reader: &mut dyn EntryReader,
    table: &BTreeMap<PackagePath, CheckedEntry>,
    manifest: &Manifest,
    operation: &mut Operation<'_>,
) -> Result<ExpandedPackage, AssetFault> {
    let mut templates = BTreeMap::new();

    for declaration in manifest.templates() {
        let entry = table
            .get(declaration.path())
            .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingEntry, LoadStage::Expansion))?;

        let content = reader.read_entry(
            entry.index,
            entry.declared_size,
            LoadStage::Expansion,
            operation,
        )?;

        if ContentDigest::of(&content) != declaration.digest() {
            return Err(AssetFault::new(
                AssetFaultKind::HashMismatch,
                LoadStage::Expansion,
            ));
        }

        let encoding = TemplateEncoding::identify(&content).ok_or_else(|| {
            AssetFault::new(
                AssetFaultKind::UnsupportedContentEncoding,
                LoadStage::Expansion,
            )
        })?;

        let template = TemplateSource::new(TemplateSourceRequest {
            id: declaration.id().clone(),
            encoding,
            extent: declaration.extent(),
            space: declaration.space(),
            defaults: declaration.defaults(),
            content,
        })
        .map_err(from_vision)?;

        if templates
            .insert(declaration.id().clone(), template)
            .is_some()
        {
            return Err(AssetFault::new(
                AssetFaultKind::DuplicateIdentity,
                LoadStage::Expansion,
            ));
        }
    }

    let mut ocr_models = BTreeMap::new();
    for declaration in manifest.ocr_models() {
        let detector = expand_model_component(reader, table, declaration.detector(), operation)?;
        let recognizer =
            expand_model_component(reader, table, declaration.recognizer(), operation)?;
        let source = OcrModelSource::new(OcrModelSourceRequest {
            model: declaration.id().clone(),
            profile: declaration.profile().clone(),
            detector,
            detector_identity: ModelComponentIdentity::new(
                declaration.detector().byte_len(),
                *declaration.detector().digest().as_bytes(),
            ),
            recognizer,
            recognizer_identity: ModelComponentIdentity::new(
                declaration.recognizer().byte_len(),
                *declaration.recognizer().digest().as_bytes(),
            ),
        })
        .map_err(from_ocr)?;
        if ocr_models
            .insert(declaration.id().clone(), source)
            .is_some()
        {
            return Err(AssetFault::new(
                AssetFaultKind::DuplicateIdentity,
                LoadStage::Expansion,
            ));
        }
    }

    Ok((templates, ocr_models))
}

fn expand_model_component(
    reader: &mut dyn EntryReader,
    table: &BTreeMap<PackagePath, CheckedEntry>,
    declaration: &OcrComponentDeclaration,
    operation: &mut Operation<'_>,
) -> Result<Arc<[u8]>, AssetFault> {
    let entry = table
        .get(declaration.path())
        .ok_or_else(|| AssetFault::new(AssetFaultKind::MissingEntry, LoadStage::Expansion))?;
    if entry.declared_size != declaration.byte_len() {
        return Err(AssetFault::new(
            AssetFaultKind::InvalidOcrModelMetadata,
            LoadStage::Expansion,
        ));
    }
    reader.read_entry(
        entry.index,
        entry.declared_size,
        LoadStage::Expansion,
        operation,
    )
}

const fn from_ocr(fault: OcrFault) -> AssetFault {
    let kind = match fault {
        OcrFault::ModelDigestMismatch => AssetFaultKind::HashMismatch,
        _ => AssetFaultKind::InvalidOcrModelMetadata,
    };
    AssetFault::new(kind, LoadStage::Expansion)
}

fn checkpoint(operation: &mut Operation<'_>, stage: LoadStage) -> Result<(), AssetFault> {
    operation
        .checkpoint()
        .map_err(|interruption| AssetFault::interrupted(interruption, stage))
}

const fn fault(kind: AssetFaultKind) -> AssetFault {
    AssetFault::new(kind, LoadStage::EntryMetadata)
}

#[cfg(test)]
mod tests {
    use super::{exceeds_ratio, validate_entries};
    use crate::fault::AssetFaultKind;
    use crate::limits::AssetLimits;
    use crate::reader::{EntryKind, EntryStorage, RawEntry};

    fn entry(name: &str, declared: u64, compressed: u64) -> RawEntry {
        RawEntry {
            name: name.as_bytes().to_vec(),
            kind: EntryKind::Regular,
            storage: EntryStorage::Accepted,
            declared_size: declared,
            compressed_size: compressed,
        }
    }

    #[test]
    fn the_ratio_check_multiplies_rather_than_divides() {
        let limits = AssetLimits::ceiling();

        assert!(
            !exceeds_ratio(0, 0, limits),
            "an archive of empty entries expands to nothing, which is not a bomb"
        );
        assert!(
            exceeds_ratio(1, 0, limits),
            "expansion out of no compressed bytes at all is a bomb"
        );
        assert!(!exceeds_ratio(64, 1, limits), "exactly at the ratio");
        assert!(exceeds_ratio(65, 1, limits), "one past the ratio");
        assert!(
            !exceeds_ratio(u64::MAX, u64::MAX, limits),
            "saturating the allowance must not wrap it into a refusal or a pass"
        );
    }

    #[test]
    fn a_declared_size_cannot_wrap_the_running_total_into_a_passing_value() {
        // Two entries whose declared sizes sum past u64::MAX. Unchecked
        // arithmetic would wrap the running total to a small number and admit
        // the package.
        let raw = vec![
            entry("a.bin", u64::MAX, 1),
            entry("b.bin", u64::MAX, 1),
            entry("c.bin", 2, 1),
        ];

        let fault = validate_entries(&raw, AssetLimits::ceiling(), true).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    }

    #[test]
    fn the_manifest_is_bounded_by_its_own_ceiling_not_the_per_entry_one() {
        // 4 MiB + 1 is far below the 64 MiB per-entry ceiling, so only the
        // manifest ceiling can refuse it.
        let raw = vec![entry(
            crate::manifest::MANIFEST_PATH,
            AssetLimits::MAX_MANIFEST_BYTES + 1,
            1,
        )];

        let fault = validate_entries(&raw, AssetLimits::ceiling(), true).expect_err("refused");

        assert_eq!(fault.kind(), AssetFaultKind::ArchiveLimit);
    }

    #[test]
    fn a_non_archive_source_is_not_held_to_the_ratio() {
        // A directory has no compressed representation, so its declared
        // "compressed" size is its length and the ratio is always one.
        let raw = vec![entry("a.bin", 4_096, 4_096)];

        assert!(validate_entries(&raw, AssetLimits::ceiling(), false).is_ok());
    }
}
