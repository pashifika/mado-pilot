//! Architecture inventory and dependency-direction checking for the MadoPilot
//! workspace.
//!
//! The checker separates its rules from its input acquisition so that the rules
//! can be tested without running Cargo:
//!
//! - [`graph`] holds the normalized package-graph model, the required package
//!   inventory, the deferred-package table, the Phase 0 metadata contract, and the
//!   pure validator.
//! - [`manifest`] reads the manifest-text facts Cargo metadata cannot report,
//!   namely the lint opt-in and explicit workspace inheritance.
//! - [`metadata`] holds the Cargo process adapter that turns `cargo metadata`
//!   output and the on-disk manifests into a [`metadata::WorkspaceObservation`].
//!
//! This package is repository maintenance tooling. No product package may depend
//! on it.

pub mod graph;
pub mod manifest;
pub mod metadata;
