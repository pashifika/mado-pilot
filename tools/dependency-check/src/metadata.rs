//! Cargo process adapter.
//!
//! This module is the only part of the checker that runs a subprocess or touches
//! the filesystem. It converts `cargo metadata` output plus the on-disk manifests
//! into the normalized [`PackageGraph`] and metadata that
//! [`crate::graph::validate`] and [`crate::graph::validate_metadata`] consume, and
//! probes the reserved deferred-adapter directories.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::graph::{
    DEFERRED_PACKAGES, DependencyKind, ObservedEdge, ObservedMetadata, ObservedPackage,
    ObservedWorkspaceMetadata, PackageGraph, Violation, normalize_directory,
};
use crate::manifest::{Manifest, TOOLCHAIN_TABLE, WORKSPACE_PACKAGE_TABLE};

/// File that pins the tested toolchain, relative to the workspace root.
pub const TOOLCHAIN_FILE: &str = "rust-toolchain.toml";

/// A failure that prevented the checker from inspecting the workspace.
///
/// These are tool failures rather than architecture violations.
#[derive(Debug)]
pub enum MetadataError {
    /// `cargo metadata` could not be started.
    Spawn(std::io::Error),
    /// `cargo metadata` ran but reported a failure.
    CargoFailed {
        /// Exit status, when the process exited normally.
        status: Option<i32>,
        /// Captured standard error, trimmed.
        stderr: String,
    },
    /// `cargo metadata` produced output the checker could not interpret.
    Parse(serde_json::Error),
    /// A workspace member manifest is not inside the workspace root.
    ManifestOutsideWorkspace {
        /// Manifest path reported by Cargo.
        manifest_path: PathBuf,
        /// Workspace root reported by Cargo.
        workspace_root: PathBuf,
    },
    /// A workspace member manifest could not be read.
    ManifestUnreadable {
        /// Manifest path reported by Cargo.
        manifest_path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// The toolchain pin file exists but could not be read.
    ///
    /// An absent pin file is a policy violation rather than a tool failure, so it
    /// is reported as a missing channel instead of this error.
    ToolchainUnreadable {
        /// Path the checker tried to read.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
}

impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(source) => {
                write!(formatter, "could not run `cargo metadata`: {source}")
            }
            Self::CargoFailed { status, stderr } => {
                let status = status.map_or_else(|| "signal".to_owned(), |code| code.to_string());
                write!(
                    formatter,
                    "`cargo metadata` failed with status {status}: {stderr}"
                )
            }
            Self::Parse(source) => {
                write!(
                    formatter,
                    "could not interpret `cargo metadata` output: {source}"
                )
            }
            Self::ManifestOutsideWorkspace {
                manifest_path,
                workspace_root,
            } => write!(
                formatter,
                "workspace member manifest `{}` is not inside workspace root `{}`",
                manifest_path.display(),
                workspace_root.display()
            ),
            Self::ManifestUnreadable {
                manifest_path,
                source,
            } => write!(
                formatter,
                "could not read manifest `{}`: {source}",
                manifest_path.display()
            ),
            Self::ToolchainUnreadable { path, source } => write!(
                formatter,
                "could not read toolchain pin `{}`: {source}",
                path.display()
            ),
        }
    }
}

impl Error for MetadataError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(source) => Some(source),
            Self::Parse(source) => Some(source),
            Self::ManifestUnreadable { source, .. } | Self::ToolchainUnreadable { source, .. } => {
                Some(source)
            }
            Self::CargoFailed { .. } | Self::ManifestOutsideWorkspace { .. } => None,
        }
    }
}

/// The workspace as observed through Cargo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceObservation {
    /// Absolute workspace root reported by Cargo.
    pub workspace_root: PathBuf,
    /// Normalized package graph of the workspace members.
    pub graph: PackageGraph,
    /// Shared metadata declared by the root workspace manifest and the toolchain
    /// pin.
    pub workspace: ObservedWorkspaceMetadata,
    /// Shared manifest metadata for every member.
    pub members: Vec<ObservedMetadata>,
    /// Violations that only the Cargo adapter can see, because they concern how a
    /// dependency resolves rather than which packages it connects.
    pub source_violations: Vec<Violation>,
}

/// Reads the workspace graph by running `cargo metadata`.
///
/// `manifest_path` selects the workspace manifest; when it is `None`, Cargo
/// resolves the workspace from the current directory.
///
/// # Errors
///
/// Returns a [`MetadataError`] when Cargo cannot be started, exits with a
/// failure, produces output the checker cannot interpret, or reports a member
/// manifest outside the workspace root.
pub fn read_workspace(manifest_path: Option<&Path>) -> Result<WorkspaceObservation, MetadataError> {
    let mut command = Command::new(cargo_program());
    command.args(["metadata", "--format-version", "1", "--no-deps"]);
    if let Some(manifest_path) = manifest_path {
        command.arg("--manifest-path").arg(manifest_path);
    }

    let output = command.output().map_err(MetadataError::Spawn)?;
    if !output.status.success() {
        return Err(MetadataError::CargoFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    read_metadata_output(&output.stdout)
}

/// Reads a workspace observation from `cargo metadata --format-version 1` output.
///
/// [`read_workspace`] runs Cargo and passes its standard output here. The
/// separation exists so that the adapter's path, source, and manifest policies can
/// be exercised against synthetic Cargo output over controlled directories, which
/// is the only way to reach the failure branches deterministically.
///
/// The member manifests, the root workspace manifest, and the toolchain pin are
/// still read from disk, because Cargo does not report the facts they carry.
///
/// # Errors
///
/// Returns a [`MetadataError`] when the output cannot be interpreted, a member
/// manifest lies outside the workspace root or cannot be read, or the toolchain pin
/// exists but cannot be read.
pub fn read_metadata_output(output: &[u8]) -> Result<WorkspaceObservation, MetadataError> {
    let metadata: CargoMetadata = serde_json::from_slice(output).map_err(MetadataError::Parse)?;
    observation_from_metadata(metadata)
}

/// Reports reserved deferred-adapter directories that exist under
/// `workspace_root`.
///
/// A deferred adapter must not exist even as an empty directory, because an empty
/// reserved directory reads as a promised adapter.
///
/// This uses `symlink_metadata` rather than `exists` so that a dangling symlink at
/// a reserved path is still reported: `Path::exists` follows symlinks and returns
/// `false` for a broken one, which would let a tracked broken link occupy a
/// reserved adapter path unnoticed.
#[must_use]
pub fn deferred_directory_violations(workspace_root: &Path) -> Vec<Violation> {
    DEFERRED_PACKAGES
        .iter()
        .filter(|deferred| {
            std::fs::symlink_metadata(workspace_root.join(deferred.directory)).is_ok()
        })
        .map(|deferred| Violation::DeferredDirectory {
            directory: deferred.directory.to_owned(),
            reason: deferred.reason.to_owned(),
        })
        .collect()
}

fn cargo_program() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn observation_from_metadata(
    metadata: CargoMetadata,
) -> Result<WorkspaceObservation, MetadataError> {
    let workspace_root = PathBuf::from(&metadata.workspace_root);

    let mut member_directories: BTreeMap<&str, String> = BTreeMap::new();
    for package in &metadata.packages {
        member_directories.insert(
            package.name.as_str(),
            manifest_directory(Path::new(&package.manifest_path), &workspace_root)?,
        );
    }
    let member_paths: BTreeSet<&str> = member_directories.values().map(String::as_str).collect();

    let mut packages = Vec::with_capacity(metadata.packages.len());
    let mut members = Vec::with_capacity(metadata.packages.len());
    let mut edges = Vec::new();
    let mut violations = Vec::new();

    for package in &metadata.packages {
        let manifest_path = PathBuf::from(&package.manifest_path);
        let directory = manifest_directory(&manifest_path, &workspace_root)?;
        packages.push(ObservedPackage::new(
            package.name.clone(),
            directory.clone(),
        ));

        let manifest = Manifest::parse(&read_manifest(&manifest_path)?);
        members.push(ObservedMetadata {
            name: package.name.clone(),
            directory,
            version: package.version.clone(),
            edition: package.edition.clone(),
            rust_version: package.rust_version.clone(),
            license: package.license.clone(),
            repository: package.repository.clone(),
            // Cargo reports `publish: null` when publication is unrestricted and
            // `publish: []` when `publish = false`.
            publishable: package
                .publish
                .as_ref()
                .is_none_or(|registries| !registries.is_empty()),
            inherited_fields: manifest.inherited_package_fields(),
            inherits_workspace_lints: manifest.inherits_workspace_lints(),
        });

        for dependency in &package.dependencies {
            let kind = dependency_kind(dependency.kind.as_deref());
            // Cargo reports the real package in `name` and the manifest-visible
            // alias of a `package = "..."` rename in `rename`. Both matter: the
            // visible name is what Rust source imports, and the real name is what
            // Cargo builds.
            let visible = dependency.rename.as_deref().unwrap_or(&dependency.name);

            // A path dependency must resolve to a workspace member. Anything else
            // would let product code enter the build without appearing in the
            // documented inventory.
            let resolved = match &dependency.path {
                Some(path) => {
                    let relative =
                        manifest_directory(&Path::new(path).join("Cargo.toml"), &workspace_root)
                            .ok()
                            .filter(|relative| member_paths.contains(relative.as_str()));
                    if relative.is_none() {
                        violations.push(Violation::NonMemberPathDependency {
                            from: package.name.clone(),
                            dependency: visible.to_owned(),
                            path: path.clone(),
                        });
                        continue;
                    }
                    relative
                }
                None => None,
            };

            // A dependency claims a member when either the visible name or the real
            // package name is a member name. The visible name catches a renamed
            // external crate masquerading as an internal contract package; the real
            // name catches a same-named crate pulled from a registry or Git source.
            let claimed = if member_directories.contains_key(visible) {
                Some(visible)
            } else if member_directories.contains_key(dependency.name.as_str()) {
                Some(dependency.name.as_str())
            } else {
                None
            };
            let Some(member) = claimed else {
                continue;
            };

            // The claim holds only when the real package is that member and its path
            // resolves to that member's own directory. Otherwise the dependency is
            // satisfied by something other than the inventory package it names.
            let directory = &member_directories[member];
            if dependency.name != member || resolved.as_deref() != Some(directory.as_str()) {
                violations.push(Violation::ShadowedMemberDependency {
                    from: package.name.clone(),
                    to: member.to_owned(),
                    kind,
                });
                continue;
            }

            // Graph edges use the real package name, which is the package the
            // architecture allowlist is written against.
            edges.push(ObservedEdge {
                from: package.name.clone(),
                to: dependency.name.clone(),
                kind,
            });
        }
    }

    let workspace = workspace_metadata(&workspace_root)?;

    Ok(WorkspaceObservation {
        workspace_root,
        graph: PackageGraph::new(packages, edges),
        workspace,
        members,
        source_violations: violations,
    })
}

/// Reads the shared metadata declared by the root workspace manifest and the
/// toolchain pin.
///
/// Cargo reports resolved member values, so the root declaration is the only place
/// the checker can see what members are supposed to inherit.
fn workspace_metadata(workspace_root: &Path) -> Result<ObservedWorkspaceMetadata, MetadataError> {
    let root_manifest = workspace_root.join("Cargo.toml");
    let manifest = Manifest::parse(&read_manifest(&root_manifest)?);
    let shared = |field: &str| {
        manifest
            .string(WORKSPACE_PACKAGE_TABLE, field)
            .map(str::to_owned)
    };

    let pin = workspace_root.join(TOOLCHAIN_FILE);
    let toolchain_channel = match std::fs::read_to_string(&pin) {
        Ok(text) => Manifest::parse(&text)
            .string(TOOLCHAIN_TABLE, "channel")
            .map(str::to_owned),
        // A missing pin leaves the contributor toolchain unpinned, which the
        // contract rejects; that is a violation, not a tool failure.
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => return Err(MetadataError::ToolchainUnreadable { path: pin, source }),
    };

    Ok(ObservedWorkspaceMetadata {
        version: shared("version"),
        edition: shared("edition"),
        rust_version: shared("rust-version"),
        license: shared("license"),
        repository: shared("repository"),
        toolchain_channel,
    })
}

fn read_manifest(manifest_path: &Path) -> Result<String, MetadataError> {
    std::fs::read_to_string(manifest_path).map_err(|source| MetadataError::ManifestUnreadable {
        manifest_path: manifest_path.to_path_buf(),
        source,
    })
}

fn manifest_directory(
    manifest_path: &Path,
    workspace_root: &Path,
) -> Result<String, MetadataError> {
    let directory =
        manifest_path
            .parent()
            .ok_or_else(|| MetadataError::ManifestOutsideWorkspace {
                manifest_path: manifest_path.to_path_buf(),
                workspace_root: workspace_root.to_path_buf(),
            })?;
    let relative = directory.strip_prefix(workspace_root).map_err(|_| {
        MetadataError::ManifestOutsideWorkspace {
            manifest_path: manifest_path.to_path_buf(),
            workspace_root: workspace_root.to_path_buf(),
        }
    })?;

    Ok(normalize_directory(&relative.to_string_lossy()))
}

/// Build dependencies ship with the product's build, so they are treated as
/// production edges. Only development dependencies are exempt from the
/// production rules.
fn dependency_kind(kind: Option<&str>) -> DependencyKind {
    match kind {
        Some("dev") => DependencyKind::Development,
        _ => DependencyKind::Production,
    }
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_root: String,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    manifest_path: String,
    version: String,
    edition: String,
    rust_version: Option<String>,
    license: Option<String>,
    repository: Option<String>,
    /// `None` when publication is unrestricted; `Some([])` for `publish = false`.
    publish: Option<Vec<String>>,
    dependencies: Vec<CargoDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoDependency {
    name: String,
    /// Manifest-visible alias when the dependency uses `package = "..."`; `None`
    /// when the dependency is declared under its real package name.
    rename: Option<String>,
    kind: Option<String>,
    /// Present only for a path dependency; absent for registry and Git sources.
    path: Option<String>,
}
