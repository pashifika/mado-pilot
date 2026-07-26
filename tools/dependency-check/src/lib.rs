//! Architecture inventory and dependency-direction checking for the MadoPilot
//! workspace.
//!
//! The checker is split into two halves so that the rules can be tested without
//! running Cargo:
//!
//! - [`graph`] holds the normalized package-graph model, the required package
//!   inventory, the deferred-package table, and the pure validator.
//! - [`metadata`] holds the Cargo process adapter that turns `cargo metadata`
//!   output into a [`graph::PackageGraph`].
//!
//! This package is repository maintenance tooling. No product package may depend
//! on it.

pub mod graph;
pub mod metadata;
