//! The immutable package a successful load commits.
//!
//! A package exists only after every check has passed, so nothing here can
//! fail for a reason the load already covered. Resolving a template cannot
//! re-read a file, re-hash content, or consult the source again: the bytes were
//! proven at expansion and have been owned ever since, which is why a directory
//! that changes after a successful load cannot change what the package says.

use std::collections::BTreeMap;

use mado_pilot_ocr::{ModelId, OcrModelSource};
use mado_pilot_vision::{TemplateId, TemplateSource};

use crate::fault::{AssetFault, AssetFaultKind, LoadStage};
use crate::manifest::Manifest;

/// A validated, immutable asset package.
///
/// Only entries the manifest references are retained. Every other entry was
/// still name-, type-, and size-checked — an unsafe name anywhere refuses the
/// whole package — but its bytes were never expanded, so an unreferenced entry
/// costs nothing to carry and cannot reach a backend.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetPackage {
    manifest: Manifest,
    templates: BTreeMap<TemplateId, TemplateSource>,
    ocr_models: BTreeMap<ModelId, OcrModelSource>,
}

impl AssetPackage {
    pub(crate) const fn new(
        manifest: Manifest,
        templates: BTreeMap<TemplateId, TemplateSource>,
        ocr_models: BTreeMap<ModelId, OcrModelSource>,
    ) -> Self {
        Self {
            manifest,
            templates,
            ocr_models,
        }
    }

    /// Returns the validated manifest.
    #[must_use]
    pub const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Returns every template identity, in sorted order.
    ///
    /// The order is the identities' own, not the manifest's, so two packages
    /// built from the same content enumerate identically however their
    /// manifests were written.
    pub fn template_ids(&self) -> impl ExactSizeIterator<Item = &TemplateId> {
        self.templates.keys()
    }

    /// Returns the number of templates the package contains.
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    /// Resolves a validated template into a vision template source.
    ///
    /// Resolution shares the content bytes rather than copying them, so
    /// resolving the same template repeatedly costs an atomic increment. The
    /// returned value owns everything it needs and stays valid after the
    /// package is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::UnknownTemplate`] when the package contains no
    /// template with that identity.
    pub fn resolve_template(&self, id: &str) -> Result<TemplateSource, AssetFault> {
        self.templates.get(id).cloned().ok_or(AssetFault::new(
            AssetFaultKind::UnknownTemplate,
            LoadStage::Commit,
        ))
    }

    /// Returns every OCR model identity, in sorted order.
    pub fn ocr_model_ids(&self) -> impl ExactSizeIterator<Item = &ModelId> {
        self.ocr_models.keys()
    }

    /// Returns the number of OCR models the package contains.
    #[must_use]
    pub fn ocr_model_count(&self) -> usize {
        self.ocr_models.len()
    }

    /// Resolves a validated immutable OCR model source.
    ///
    /// Resolution shares both component allocations and stays valid after this
    /// package is dropped. It never re-reads or re-hashes the package source.
    ///
    /// # Errors
    ///
    /// Returns [`AssetFaultKind::UnknownOcrModel`] when no model has that identity.
    pub fn resolve_ocr_model(&self, id: &str) -> Result<OcrModelSource, AssetFault> {
        self.ocr_models.get(id).cloned().ok_or(AssetFault::new(
            AssetFaultKind::UnknownOcrModel,
            LoadStage::Commit,
        ))
    }
}
