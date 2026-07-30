//! Keeps the architecture baseline's contract values from going stale.
//!
//! The root manifest, the `REQUIRED_*` constants, and `rust-toolchain.toml` are
//! checked against each other by the architecture checker, so a contract bump that
//! misses one of them already fails. Nothing reads `docs/architecture.md`, which
//! makes it the one contract location with no mechanical guard: its value table can
//! drift while every gate passes, and that table is exactly what the checker's own
//! `UnexpectedWorkspaceMetadata` diagnostic tells the next contributor to consult.
//!
//! This is the only case in the suite that reads a repository file, which is why it
//! lives apart from the synthetic-input suites in `metadata_policy.rs`.

use std::fs;
use std::path::PathBuf;

use mado_pilot_dependency_check::graph::{
    REQUIRED_EDITION, REQUIRED_LICENSE, REQUIRED_REPOSITORY, REQUIRED_RUST_VERSION,
    REQUIRED_VERSION,
};

/// Workspace-relative location of the architecture baseline, for diagnostics.
const ARCHITECTURE_BASELINE: &str = "docs/architecture.md";

/// Resolves the baseline from the crate directory rather than the process working
/// directory, so the test passes wherever the test binary is invoked from.
fn architecture_baseline() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("..");
    path.push("..");
    path.push("docs");
    path.push("architecture.md");
    path
}

#[test]
fn every_contract_value_is_documented_in_the_architecture_baseline() {
    let path = architecture_baseline();
    let document = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "`{ARCHITECTURE_BASELINE}` must be readable at `{}`: {error}",
            path.display()
        )
    });

    let contract = [
        (stringify!(REQUIRED_VERSION), REQUIRED_VERSION),
        (stringify!(REQUIRED_EDITION), REQUIRED_EDITION),
        (stringify!(REQUIRED_RUST_VERSION), REQUIRED_RUST_VERSION),
        (stringify!(REQUIRED_LICENSE), REQUIRED_LICENSE),
        (stringify!(REQUIRED_REPOSITORY), REQUIRED_REPOSITORY),
    ];

    for (constant, value) in contract {
        // Each value is searched for as inline code anywhere in the document. The
        // document writes every contract value that way, and the narrower needle is
        // what keeps the check meaningful for values that also read as ordinary
        // prose: `2024` is a plausible year, and the licensing section already names
        // Apache-2.0 in running text, so a bare substring would keep passing after
        // the value table had drifted. Searching the whole document rather than a
        // table cell keeps the check independent of column layout, row order,
        // headings, and line numbers, so reformatting cannot break it.
        let documented = format!("`{value}`");
        assert!(
            document.contains(&documented),
            "`{ARCHITECTURE_BASELINE}` does not document `{constant}`: the value \
             {documented} does not appear as inline code anywhere in the document. \
             This document is the one contract location nothing else reads, so moving a \
             contract value means moving the root `Cargo.toml`, the `REQUIRED_*` constants \
             in `tools/dependency-check/src/graph.rs`, `rust-toolchain.toml`, and \
             `{ARCHITECTURE_BASELINE}` together in the same change"
        );
    }
}
