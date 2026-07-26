//! Normalized package-graph model and the pure architecture validator.
//!
//! Everything in this module is deterministic and free of process, filesystem,
//! and network access, so the repository rules can be exercised against synthetic
//! graphs.

use std::collections::BTreeSet;
use std::fmt;

/// Cargo package name of the platform-neutral core contract package.
pub const CORE: &str = "mado-pilot-core";
/// Cargo package name of the capture contract package.
pub const CAPTURE: &str = "mado-pilot-capture";
/// Cargo package name of the input contract package.
pub const INPUT: &str = "mado-pilot-input";
/// Cargo package name of the vision contract package.
pub const VISION: &str = "mado-pilot-vision";
/// Cargo package name of the OCR contract package.
pub const OCR: &str = "mado-pilot-ocr";
/// Cargo package name of the asset contract package.
pub const ASSETS: &str = "mado-pilot-assets";
/// Cargo package name of the runtime orchestration package.
pub const RUNTIME: &str = "mado-pilot-runtime";
/// Cargo package name of the Windows platform adapter package.
pub const PLATFORM_WINDOWS: &str = "mado-pilot-platform-windows";
/// Cargo package name of the macOS platform adapter package.
pub const PLATFORM_MACOS: &str = "mado-pilot-platform-macos";
/// Cargo package name of the OpenCV vision backend package.
pub const BACKEND_OPENCV: &str = "mado-pilot-backend-opencv";
/// Cargo package name of the ONNX Runtime OCR backend package.
pub const BACKEND_ONNX: &str = "mado-pilot-backend-onnx";
/// Cargo package name of the public Rust facade package.
pub const FACADE: &str = "mado-pilot";
/// Cargo package name of the C ABI package.
pub const CAPI: &str = "mado-pilot-capi";
/// Cargo package name of the deterministic test-support package.
pub const TESTKIT: &str = "mado-pilot-testkit";
/// Cargo package name of this maintenance package.
pub const DEPENDENCY_CHECK: &str = "mado-pilot-dependency-check";

/// Responsibility group that a workspace package belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageRole {
    /// Public Rust facade and default adapter wiring.
    Facade,
    /// Platform-neutral contract or orchestration package.
    Automation,
    /// Platform capture and input adapter.
    Platform,
    /// Vision or OCR backend adapter.
    Backend,
    /// Language-binding boundary.
    Binding,
    /// Deterministic test support.
    Support,
    /// Repository maintenance tooling.
    Maintenance,
}

impl PackageRole {
    /// Returns `true` when a package in this role ships as part of the product.
    ///
    /// Test support and maintenance tooling are not product packages, so they may
    /// never appear as a production dependency of anything.
    #[must_use]
    pub const fn is_product(self) -> bool {
        matches!(
            self,
            Self::Facade | Self::Automation | Self::Platform | Self::Backend | Self::Binding
        )
    }
}

impl fmt::Display for PackageRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Facade => "facade",
            Self::Automation => "automation",
            Self::Platform => "platform",
            Self::Backend => "backend",
            Self::Binding => "binding",
            Self::Support => "support",
            Self::Maintenance => "maintenance",
        };
        formatter.write_str(text)
    }
}

/// A package the workspace is required to contain, at a required location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredPackage {
    /// Cargo package name.
    pub name: &'static str,
    /// Directory holding the package manifest, relative to the workspace root.
    pub directory: &'static str,
    /// Responsibility group the package belongs to.
    pub role: PackageRole,
}

/// A package that is reserved conceptually but must not exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredPackage {
    /// Cargo package name the deferred adapter would use.
    pub name: &'static str,
    /// Directory the deferred adapter would occupy.
    pub directory: &'static str,
    /// Why the package is deferred.
    pub reason: &'static str,
}

/// The exact Phase 0 workspace inventory.
///
/// The fourteen product packages plus this maintenance package are the complete
/// set of workspace members. Adding a member requires updating this table and the
/// architecture baseline in the same change.
pub const REQUIRED_PACKAGES: &[RequiredPackage] = &[
    RequiredPackage {
        name: FACADE,
        directory: "crates/mado-pilot",
        role: PackageRole::Facade,
    },
    RequiredPackage {
        name: CORE,
        directory: "crates/automation/core",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: CAPTURE,
        directory: "crates/automation/capture",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: INPUT,
        directory: "crates/automation/input",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: VISION,
        directory: "crates/automation/vision",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: OCR,
        directory: "crates/automation/ocr",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: RUNTIME,
        directory: "crates/automation/runtime",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: ASSETS,
        directory: "crates/automation/assets",
        role: PackageRole::Automation,
    },
    RequiredPackage {
        name: PLATFORM_WINDOWS,
        directory: "crates/platform/windows",
        role: PackageRole::Platform,
    },
    RequiredPackage {
        name: PLATFORM_MACOS,
        directory: "crates/platform/macos",
        role: PackageRole::Platform,
    },
    RequiredPackage {
        name: BACKEND_OPENCV,
        directory: "crates/backend/opencv",
        role: PackageRole::Backend,
    },
    RequiredPackage {
        name: BACKEND_ONNX,
        directory: "crates/backend/onnx",
        role: PackageRole::Backend,
    },
    RequiredPackage {
        name: CAPI,
        directory: "crates/bindings/capi",
        role: PackageRole::Binding,
    },
    RequiredPackage {
        name: TESTKIT,
        directory: "crates/support/testkit",
        role: PackageRole::Support,
    },
    RequiredPackage {
        name: DEPENDENCY_CHECK,
        directory: "tools/dependency-check",
        role: PackageRole::Maintenance,
    },
];

/// Adapters that are reserved for future work and must not exist as packages or
/// empty package directories.
pub const DEFERRED_PACKAGES: &[DeferredPackage] = &[
    DeferredPackage {
        name: "mado-pilot-platform-adb",
        directory: "crates/platform/adb",
        reason: "ADB capture and touch input remain future work",
    },
    DeferredPackage {
        name: "mado-pilot-platform-browser",
        directory: "crates/platform/browser",
        reason: "browser and CDP targets remain future work",
    },
    DeferredPackage {
        name: "mado-pilot-backend-apple-vision",
        directory: "crates/backend/apple-vision",
        reason: "Apple Vision OCR remains future work",
    },
];

/// The exact source-to-allowed-destinations dependency allowlist.
///
/// An actual graph may omit any of these edges. It may not contain an edge that
/// is absent from this table.
pub const ALLOWED_DEPENDENCIES: &[(&str, &[&str])] = &[
    (CORE, &[]),
    (CAPTURE, &[CORE]),
    (INPUT, &[CORE]),
    (VISION, &[CORE, CAPTURE]),
    (OCR, &[CORE, CAPTURE, VISION]),
    (ASSETS, &[CORE, VISION, OCR]),
    (RUNTIME, &[CORE, CAPTURE, INPUT, VISION, OCR, ASSETS]),
    (PLATFORM_WINDOWS, &[CORE, CAPTURE, INPUT]),
    (PLATFORM_MACOS, &[CORE, CAPTURE, INPUT]),
    (BACKEND_OPENCV, &[CORE, VISION]),
    (BACKEND_ONNX, &[CORE, VISION, OCR]),
    (
        FACADE,
        &[
            RUNTIME,
            PLATFORM_WINDOWS,
            PLATFORM_MACOS,
            BACKEND_OPENCV,
            BACKEND_ONNX,
        ],
    ),
    (CAPI, &[FACADE]),
    (TESTKIT, &[CORE, CAPTURE, INPUT, VISION, OCR]),
    (DEPENDENCY_CHECK, &[]),
];

/// Returns the required-inventory entry for `name`, if the workspace expects it.
#[must_use]
pub fn required_package(name: &str) -> Option<&'static RequiredPackage> {
    REQUIRED_PACKAGES.iter().find(|entry| entry.name == name)
}

/// Returns the deferred-adapter entry for `name`, if the name is reserved.
#[must_use]
pub fn deferred_package(name: &str) -> Option<&'static DeferredPackage> {
    DEFERRED_PACKAGES.iter().find(|entry| entry.name == name)
}

/// Returns the deferred-adapter entry that owns `directory`, if any.
#[must_use]
pub fn deferred_package_for_directory(directory: &str) -> Option<&'static DeferredPackage> {
    let normalized = normalize_directory(directory);
    DEFERRED_PACKAGES
        .iter()
        .find(|entry| entry.directory == normalized)
}

/// Returns the allowed MadoPilot destinations for `name`.
///
/// Returns `None` when `name` is not part of the required inventory.
#[must_use]
pub fn allowed_dependencies(name: &str) -> Option<&'static [&'static str]> {
    ALLOWED_DEPENDENCIES
        .iter()
        .find(|(source, _)| *source == name)
        .map(|(_, destinations)| *destinations)
}

/// How a dependency edge is declared in a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DependencyKind {
    /// A normal or build dependency, which ships with the product.
    Production,
    /// A development dependency, which is used only by tests, benches, and
    /// examples.
    Development,
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Production => "production",
            Self::Development => "development",
        };
        formatter.write_str(text)
    }
}

/// A workspace package as observed in the actual repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedPackage {
    /// Cargo package name.
    pub name: String,
    /// Directory holding the manifest, relative to the workspace root, using `/`
    /// separators.
    pub directory: String,
}

impl ObservedPackage {
    /// Creates an observed package, normalizing the directory separators.
    #[must_use]
    pub fn new(name: impl Into<String>, directory: impl AsRef<str>) -> Self {
        Self {
            name: name.into(),
            directory: normalize_directory(directory.as_ref()),
        }
    }
}

/// A dependency edge between two workspace packages as observed in the actual
/// repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedEdge {
    /// Package that declares the dependency.
    pub from: String,
    /// Workspace package that is depended upon.
    pub to: String,
    /// How the dependency is declared.
    pub kind: DependencyKind,
}

impl ObservedEdge {
    /// Creates a production dependency edge.
    #[must_use]
    pub fn production(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: DependencyKind::Production,
        }
    }

    /// Creates a development dependency edge.
    #[must_use]
    pub fn development(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: DependencyKind::Development,
        }
    }
}

/// A normalized view of the workspace package graph.
///
/// Normalization sorts and deduplicates packages and edges so that validation
/// results and diagnostics do not depend on Cargo's output ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageGraph {
    packages: Vec<ObservedPackage>,
    edges: Vec<ObservedEdge>,
}

impl PackageGraph {
    /// Builds a normalized graph from observed packages and edges.
    #[must_use]
    pub fn new(
        packages: impl IntoIterator<Item = ObservedPackage>,
        edges: impl IntoIterator<Item = ObservedEdge>,
    ) -> Self {
        let mut packages: Vec<ObservedPackage> = packages
            .into_iter()
            .map(|package| ObservedPackage::new(package.name, package.directory))
            .collect();
        packages.sort();
        packages.dedup();

        let mut edges: Vec<ObservedEdge> = edges.into_iter().collect();
        edges.sort();
        edges.dedup();

        Self { packages, edges }
    }

    /// Returns the normalized packages, sorted by name and directory.
    #[must_use]
    pub fn packages(&self) -> &[ObservedPackage] {
        &self.packages
    }

    /// Returns the normalized edges, sorted by source, destination, and kind.
    #[must_use]
    pub fn edges(&self) -> &[ObservedEdge] {
        &self.edges
    }
}

/// A single architecture rule violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A package the inventory requires is not present in the workspace.
    MissingPackage {
        /// Required package name.
        name: String,
        /// Directory the package is expected to occupy.
        expected_directory: String,
    },
    /// A workspace member that the inventory does not recognize.
    UnexpectedPackage {
        /// Observed package name.
        name: String,
        /// Observed directory.
        directory: String,
    },
    /// A required package exists outside its documented responsibility group.
    MisplacedPackage {
        /// Required package name.
        name: String,
        /// Directory the package is expected to occupy.
        expected_directory: String,
        /// Directory the package actually occupies.
        actual_directory: String,
    },
    /// A deferred adapter has been added as a workspace package.
    DeferredPackage {
        /// Observed package name.
        name: String,
        /// Observed directory.
        directory: String,
        /// Why the adapter is deferred.
        reason: String,
    },
    /// A deferred adapter directory exists even though no package may occupy it.
    DeferredDirectory {
        /// Reserved directory that exists.
        directory: String,
        /// Why the adapter is deferred.
        reason: String,
    },
    /// A dependency edge is absent from the allowlist.
    ForbiddenDependency {
        /// Package that declares the dependency.
        from: String,
        /// Workspace package that is depended upon.
        to: String,
        /// How the dependency is declared.
        kind: DependencyKind,
        /// Destinations the source package is allowed to depend on.
        allowed: Vec<String>,
    },
    /// A product package takes a production dependency on test support.
    TestSupportInProduction {
        /// Package that declares the dependency.
        from: String,
    },
    /// A package depends on repository maintenance tooling.
    MaintenanceToolDependency {
        /// Package that declares the dependency.
        from: String,
        /// How the dependency is declared.
        kind: DependencyKind,
    },
    /// A dependency carries a workspace member's name but resolves from a
    /// registry or Git source instead of the member itself.
    ShadowedMemberDependency {
        /// Package that declares the dependency.
        from: String,
        /// Workspace member name the dependency claims.
        to: String,
        /// How the dependency is declared.
        kind: DependencyKind,
    },
    /// A path dependency points at something that is not a workspace member.
    NonMemberPathDependency {
        /// Package that declares the dependency.
        from: String,
        /// Dependency name.
        dependency: String,
        /// Path the dependency points at, as reported by Cargo.
        path: String,
    },
    /// A Phase 0 package is publishable.
    PublishablePackage {
        /// Package name.
        name: String,
    },
    /// A package declares an edition other than the workspace edition.
    UnexpectedEdition {
        /// Package name.
        name: String,
        /// Edition the package declares.
        edition: String,
        /// Edition the workspace requires.
        expected: String,
    },
    /// A package declares a license other than the project license.
    UnexpectedLicense {
        /// Package name.
        name: String,
        /// License the package declares, if any.
        license: Option<String>,
        /// License the project requires.
        expected: String,
    },
    /// A package overrides shared metadata that every member must agree on.
    InconsistentMetadata {
        /// Package name.
        name: String,
        /// Manifest field that disagrees.
        field: &'static str,
        /// Value the package declares, if any.
        value: Option<String>,
        /// Value the reference package declares, if any.
        expected: Option<String>,
        /// Package whose value is treated as the reference.
        reference: String,
    },
    /// A member does not opt into the workspace lint policy.
    MissingWorkspaceLints {
        /// Package name.
        name: String,
        /// Directory holding the manifest.
        directory: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPackage {
                name,
                expected_directory,
            } => write!(
                formatter,
                "required package `{name}` is missing; expected a workspace member at `{expected_directory}`"
            ),
            Self::UnexpectedPackage { name, directory } => write!(
                formatter,
                "unexpected workspace package `{name}` at `{directory}`; add it to the Phase 0 inventory in `docs/architecture.md` and to `REQUIRED_PACKAGES`, or remove it"
            ),
            Self::MisplacedPackage {
                name,
                expected_directory,
                actual_directory,
            } => write!(
                formatter,
                "package `{name}` is at `{actual_directory}` but its documented responsibility group places it at `{expected_directory}`"
            ),
            Self::DeferredPackage {
                name,
                directory,
                reason,
            } => write!(
                formatter,
                "deferred package `{name}` at `{directory}` must not exist: {reason}; add it only with an owner, an implemented contract, tests, and an explicit support statement"
            ),
            Self::DeferredDirectory { directory, reason } => write!(
                formatter,
                "reserved directory `{directory}` must not exist: {reason}"
            ),
            Self::ForbiddenDependency {
                from,
                to,
                kind,
                allowed,
            } => {
                let allowed = if allowed.is_empty() {
                    "no MadoPilot package".to_owned()
                } else {
                    allowed.join(", ")
                };
                write!(
                    formatter,
                    "forbidden {kind} dependency `{from}` -> `{to}`; `{from}` may depend on {allowed}"
                )
            }
            Self::TestSupportInProduction { from } => write!(
                formatter,
                "production package `{from}` takes a production dependency on `{TESTKIT}`; move it to `[dev-dependencies]`"
            ),
            Self::MaintenanceToolDependency { from, kind } => write!(
                formatter,
                "package `{from}` declares a {kind} dependency on the maintenance tool `{DEPENDENCY_CHECK}`; repository tooling is never a package dependency"
            ),
            Self::ShadowedMemberDependency { from, to, kind } => write!(
                formatter,
                "`{from}` declares a {kind} dependency named `{to}` that resolves from a registry or Git source instead of the workspace member; use `{{ path = \"...\" }}` so the dependency cannot be satisfied by an unrelated crate of the same name"
            ),
            Self::NonMemberPathDependency {
                from,
                dependency,
                path,
            } => write!(
                formatter,
                "`{from}` declares a path dependency `{dependency}` at `{path}`, which is not a workspace member; product code must come from the documented inventory"
            ),
            Self::PublishablePackage { name } => write!(
                formatter,
                "package `{name}` is publishable; every Phase 0 package must declare `publish = false` until an implemented, tested package intends publication"
            ),
            Self::UnexpectedEdition {
                name,
                edition,
                expected,
            } => write!(
                formatter,
                "package `{name}` declares edition `{edition}` but the workspace requires `{expected}`; inherit it with `edition.workspace = true`"
            ),
            Self::UnexpectedLicense {
                name,
                license,
                expected,
            } => {
                let license = license.as_deref().unwrap_or("no license");
                write!(
                    formatter,
                    "package `{name}` declares `{license}` but the project license is `{expected}`; inherit it with `license.workspace = true`"
                )
            }
            Self::InconsistentMetadata {
                name,
                field,
                value,
                expected,
                reference,
            } => {
                let value = value.as_deref().unwrap_or("unset");
                let expected = expected.as_deref().unwrap_or("unset");
                write!(
                    formatter,
                    "package `{name}` declares `{field} = {value}` but `{reference}` declares `{expected}`; every member must inherit this field with `{field}.workspace = true`"
                )
            }
            Self::MissingWorkspaceLints { name, directory } => write!(
                formatter,
                "package `{name}` at `{directory}` does not opt into the workspace lint policy; add `[lints]` with `workspace = true`, otherwise the workspace Rust, rustdoc, and Clippy lints are silently disabled for this package"
            ),
        }
    }
}

/// Edition every workspace member must use.
pub const REQUIRED_EDITION: &str = "2024";
/// License every workspace member must declare, matching the root `LICENSE` file.
pub const REQUIRED_LICENSE: &str = "Apache-2.0";

/// Manifest metadata observed for one workspace member.
///
/// These are the fields the `rust-workspace-baseline` specification requires every
/// member to share. The checker reads them so that a member cannot silently
/// override inherited metadata or opt out of the lint policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMetadata {
    /// Cargo package name.
    pub name: String,
    /// Directory holding the manifest, relative to the workspace root.
    pub directory: String,
    /// Declared package version.
    pub version: String,
    /// Declared Rust edition.
    pub edition: String,
    /// Declared minimum supported Rust version, if any.
    pub rust_version: Option<String>,
    /// Declared license expression, if any.
    pub license: Option<String>,
    /// Declared repository URL, if any.
    pub repository: Option<String>,
    /// Whether Cargo would allow this package to be published.
    pub publishable: bool,
    /// Whether the manifest opts into `[workspace.lints]`.
    pub inherits_workspace_lints: bool,
}

/// Validates shared package metadata and lint opt-in for every member.
///
/// `version`, `rust-version`, and `repository` are checked for agreement rather
/// than against hard-coded values, so a release version bump does not require a
/// checker change while a member that overrides an inherited field still fails.
/// The public facade is the reference; when it is absent the inventory check
/// already reports it as missing and this check is skipped.
#[must_use]
pub fn validate_metadata(members: &[ObservedMetadata]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut members: Vec<&ObservedMetadata> = members.iter().collect();
    members.sort_by(|left, right| left.name.cmp(&right.name));

    for member in &members {
        if member.publishable {
            violations.push(Violation::PublishablePackage {
                name: member.name.clone(),
            });
        }
        if member.edition != REQUIRED_EDITION {
            violations.push(Violation::UnexpectedEdition {
                name: member.name.clone(),
                edition: member.edition.clone(),
                expected: REQUIRED_EDITION.to_owned(),
            });
        }
        if member.license.as_deref() != Some(REQUIRED_LICENSE) {
            violations.push(Violation::UnexpectedLicense {
                name: member.name.clone(),
                license: member.license.clone(),
                expected: REQUIRED_LICENSE.to_owned(),
            });
        }
        if !member.inherits_workspace_lints {
            violations.push(Violation::MissingWorkspaceLints {
                name: member.name.clone(),
                directory: member.directory.clone(),
            });
        }
    }

    let Some(reference) = members.iter().find(|member| member.name == FACADE) else {
        return violations;
    };

    for member in &members {
        if member.name == reference.name {
            continue;
        }
        let shared: [(&'static str, Option<String>, Option<String>); 3] = [
            (
                "version",
                Some(member.version.clone()),
                Some(reference.version.clone()),
            ),
            (
                "rust-version",
                member.rust_version.clone(),
                reference.rust_version.clone(),
            ),
            (
                "repository",
                member.repository.clone(),
                reference.repository.clone(),
            ),
        ];
        for (field, value, expected) in shared {
            if value != expected {
                violations.push(Violation::InconsistentMetadata {
                    name: member.name.clone(),
                    field,
                    value,
                    expected,
                    reference: reference.name.clone(),
                });
            }
        }
    }

    violations
}

/// Reports whether a manifest opts into the workspace lint policy.
///
/// Cargo metadata does not expose the `[lints]` table, so this reads the manifest
/// text and recognizes the two canonical spellings:
///
/// ```text
/// [lints]
/// workspace = true
/// ```
///
/// and the dotted `lints.workspace = true`. An exotic but valid spelling is
/// reported as missing, which fails loudly with an actionable message rather than
/// silently accepting a package whose lints are disabled.
#[must_use]
pub fn manifest_inherits_workspace_lints(manifest: &str) -> bool {
    let mut in_lints_table = false;
    for line in manifest.lines() {
        let line = match line.split_once('#') {
            Some((before, _)) => before.trim(),
            None => line.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_lints_table = line == "[lints]";
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact == "lints.workspace=true" {
            return true;
        }
        if in_lints_table && compact == "workspace=true" {
            return true;
        }
    }
    false
}

/// Validates a normalized workspace graph against the Phase 0 architecture rules.
///
/// Returns an empty vector when the graph is compliant. Otherwise the violations
/// are returned in a deterministic order: inventory findings first, then
/// dependency findings sorted by source and destination.
#[must_use]
pub fn validate(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = validate_inventory(graph);
    violations.extend(validate_dependencies(graph));
    violations
}

fn validate_inventory(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let observed: BTreeSet<&str> = graph
        .packages()
        .iter()
        .map(|package| package.name.as_str())
        .collect();

    for required in REQUIRED_PACKAGES {
        if !observed.contains(required.name) {
            violations.push(Violation::MissingPackage {
                name: required.name.to_owned(),
                expected_directory: required.directory.to_owned(),
            });
        }
    }

    for package in graph.packages() {
        if let Some(deferred) = deferred_package(&package.name)
            .or_else(|| deferred_package_for_directory(&package.directory))
        {
            violations.push(Violation::DeferredPackage {
                name: package.name.clone(),
                directory: package.directory.clone(),
                reason: deferred.reason.to_owned(),
            });
            continue;
        }

        match required_package(&package.name) {
            None => violations.push(Violation::UnexpectedPackage {
                name: package.name.clone(),
                directory: package.directory.clone(),
            }),
            Some(required) if required.directory != package.directory => {
                violations.push(Violation::MisplacedPackage {
                    name: package.name.clone(),
                    expected_directory: required.directory.to_owned(),
                    actual_directory: package.directory.clone(),
                });
            }
            Some(_) => {}
        }
    }

    violations
}

fn validate_dependencies(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();

    for edge in graph.edges() {
        if edge.from == edge.to {
            // Cargo accepts a package's dev-dependency on itself, and it says
            // nothing about architecture direction. Treating it as an edge would
            // reject a legitimate manifest.
            continue;
        }

        if edge.to == DEPENDENCY_CHECK {
            violations.push(Violation::MaintenanceToolDependency {
                from: edge.from.clone(),
                kind: edge.kind,
            });
            continue;
        }

        let Some(source) = required_package(&edge.from) else {
            // An unrecognized source package is already reported by the
            // inventory check; its edges add no actionable information.
            continue;
        };

        if edge.to == TESTKIT && edge.kind == DependencyKind::Production && source.role.is_product()
        {
            violations.push(Violation::TestSupportInProduction {
                from: edge.from.clone(),
            });
            continue;
        }

        let allowed = allowed_dependencies(&edge.from).unwrap_or(&[]);
        // Test support is available to every product package, but only as a
        // development dependency, because that is what testkit exists for.
        let development_extra = edge.kind == DependencyKind::Development
            && source.role.is_product()
            && edge.to == TESTKIT;

        if !allowed.contains(&edge.to.as_str()) && !development_extra {
            let mut allowed: Vec<String> = allowed.iter().map(|name| (*name).to_owned()).collect();
            if source.role.is_product() {
                allowed.push(format!("{TESTKIT} (development only)"));
            }
            violations.push(Violation::ForbiddenDependency {
                from: edge.from.clone(),
                to: edge.to.clone(),
                kind: edge.kind,
                allowed,
            });
        }
    }

    violations
}

/// Normalizes a manifest directory into a workspace-relative `/`-separated path.
#[must_use]
pub fn normalize_directory(directory: &str) -> String {
    let normalized = directory.replace('\\', "/");
    let trimmed = normalized
        .trim_start_matches("./")
        .trim_matches('/')
        .to_owned();
    if trimmed.is_empty() {
        ".".to_owned()
    } else {
        trimmed
    }
}
