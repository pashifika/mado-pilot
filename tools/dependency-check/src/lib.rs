//! Architecture, dependency, and release-scope checking for the MadoPilot
//! workspace.
//!
//! The checker separates pure rules from process, filesystem, and Git-tree
//! adapters:
//!
//! - [`graph`] holds the normalized package graph, inventory, metadata contract,
//!   and architecture validator.
//! - [`manifest`] reads manifest-text facts Cargo metadata cannot report.
//! - [`metadata`] adapts Cargo metadata and on-disk manifests into normalized
//!   observations.
//! - [`release`] validates the committed source-release body, CMake and public
//!   header surfaces, frozen evidence trees, Git modes, and artifact exclusions.
//!
//! This package is repository maintenance tooling. No product package may depend
//! on it.

pub mod graph;
pub mod manifest;
pub mod metadata;
pub mod release;
