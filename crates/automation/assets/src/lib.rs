//! MadoPilot asset contracts.
//!
//! # Responsibility
//!
//! This package owns the versioned asset manifest schema, manifest and entry
//! validation, deterministic and network-free loading from directory, memory,
//! and archive sources, and resolution of validated entries into vision
//! template and OCR model source descriptors.
//!
//! Loading is one pipeline, not three. A directory, a caller's memory, and a
//! local ZIP archive differ in what they can record and in what can go wrong
//! while reading them; they do not differ in what makes a package valid. The
//! same names are normalized, the same duplicates refused, the same manifest
//! parsed, and the same hashes verified, which is what lets a package be
//! shipped as a directory during development and as an archive in production
//! without becoming a different package.
//!
//! Nothing here opens a network connection, resolves a URI, downloads missing
//! content, executes package content, or writes an entry to a filesystem
//! location that a later read would treat as trusted.
//!
//! # Allowed seam
//!
//! This package may depend on the MadoPilot core, vision, and OCR contract
//! packages. Vision and OCR never depend on asset-package representations, so a
//! caller may supply direct file or memory sources without adopting the asset
//! manifest.
//!
//! # Safety ceilings
//!
//! The six ceilings on [`AssetLimits`] come from the adversarial and
//! representative measurements in `docs/evidence/g-014` and are fixed by
//! [ADR 0001]. A host may configure any limit at or below its ceiling; a limit
//! above one is rejected rather than clamped.
//!
//! Three of them describe archive structure — the recorded entry count, the
//! total source bytes, and the expansion ratio — and an archive is the only
//! source that has them. The other three bound allocation the loader performs
//! whatever the source is, so directory and memory sources are held to the
//! manifest-byte, per-entry-byte, and total-expansion limits as well. That is
//! an implementation decision this package documents, not a widening of
//! ADR 0001, which fixes the ceilings for archives and leaves directory and
//! memory containment to their own rules.
//!
//! # Implementation status
//!
//! Phase 1 template loading remains complete. Schema version 1 stays readable;
//! schema version 2 adds complete bounded OCR model/profile declarations.
//! Directory, memory, and archive sources resolve identical immutable template
//! and OCR model sources through the same `G-014`-bounded pipeline.
//!
//! **The public names here are reviewed, not yet stable.**
//! `docs/adr/0006-public-rust-names-and-compatibility-policy.md` records the
//! review that settled them and the policy that now applies: renaming or
//! removing one is a breaking change needing an ADR and a version bump, while
//! adding is free. The stability promise itself begins at 1.0.
//!
//! # Where to start
//!
//! ```no_run
//! use mado_pilot_assets::{AssetLimits, PackageLoader, PackageSource};
//! use mado_pilot_core::OperationContext;
//!
//! // A host may tighten a limit, and can never loosen one.
//! let limits = AssetLimits::ceiling().with_max_entry_count(64)?;
//! let loader = PackageLoader::with_limits(limits);
//!
//! // The same package validates identically from a directory or an archive.
//! let package = loader.load(
//!     &PackageSource::archive_file("theme.zip"),
//!     &OperationContext::new(),
//! )?;
//!
//! // Resolution hands the vision contract a template and nothing about assets.
//! let template = package.resolve_template("start-button")?;
//! assert_eq!(template.id().as_str(), "start-button");
//! # Ok::<(), mado_pilot_assets::AssetFault>(())
//! ```
//!
//! [ADR 0001]: https://github.com/pashifika/mado-pilot/blob/main/docs/adr/0001-asset-archive-container-and-safety-ceilings.md

mod archive;
mod directory;
mod filesystem;
mod memory;
mod reader;

pub mod fault;
pub mod limits;
pub mod load;
pub mod manifest;
pub mod package;
pub mod path;
pub mod source;

pub use fault::{AssetFault, AssetFaultKind, LoadStage};
pub use limits::AssetLimits;
pub use load::PackageLoader;
pub use manifest::{
    ContentDigest, HASH_ALGORITHM, MANIFEST_PATH, Manifest, OcrComponentDeclaration,
    OcrModelDeclaration, Provenance, SCHEMA_VERSION, TemplateDeclaration,
};
pub use package::AssetPackage;
pub use path::PackagePath;
pub use source::{MemoryEntry, MemoryPackage, PackageSource};
