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
//! The six safety ceilings on [`AssetLimits`] are supported by the adversarial
//! and representative measurements in `docs/evidence/g-014`. A host may
//! configure any limit at or below its ceiling; higher limits are rejected.
//!
//! Three of them describe archive structure — the recorded entry count, the
//! total source bytes, and the expansion ratio — and an archive is the only
//! source that has them. The other three bound allocation the loader performs
//! whatever the source is, so directory and memory sources are held to the
//! manifest-byte, per-entry-byte, and total-expansion limits as well.
//!
//! # Implementation status
//!
//! Phase 1 template loading remains complete. Schema version 1 stays readable;
//! schema version 2 adds complete bounded OCR model/profile declarations.
//! Directory, memory, and archive sources resolve identical immutable template
//! and OCR model sources through the same `G-014`-bounded pipeline.
//!
//! **The public names here are reviewed, not yet stable.**
//! Renaming or removing a public name is a breaking change requiring a version
//! bump; additions are allowed. The stability promise begins at 1.0.
//! Current naming and compatibility policy lives in `docs/architecture.md`.
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
