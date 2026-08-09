//! Deterministic tests for the Cargo and filesystem adapter.
//!
//! These cases exercise the policies that only the adapter can see: how a
//! dependency resolves, whether a path stays inside the inventory, how Cargo's
//! `publish` field decodes, and what happens to a reserved directory. Cargo's
//! output is supplied as synthetic `--format-version 1` JSON over a temporary
//! directory tree, so no case depends on the real repository or on running Cargo,
//! except the one that deliberately checks the command-line exit status.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Value, json};

use mado_pilot_dependency_check::graph::{CAPTURE, CORE, DependencyKind, ObservedEdge, Violation};
use mado_pilot_dependency_check::metadata::{
    MetadataError, WorkspaceObservation, deferred_directory_violations, read_metadata_output,
};

const CORE_DIRECTORY: &str = "crates/automation/core";
const CAPTURE_DIRECTORY: &str = "crates/automation/capture";

/// A temporary directory tree that is removed when the test ends.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn create(label: &str) -> Self {
        static SEQUENCE: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "mado-pilot-dependency-check-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("a temporary workspace directory can be created");
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        let parent = path
            .parent()
            .expect("a written file has a parent directory");
        fs::create_dir_all(parent).expect("a temporary parent directory can be created");
        fs::write(&path, contents).expect("a temporary file can be written");
        path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        // Best effort: a directory that resists removal must not fail a passing test.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn root_manifest() -> String {
    format!(
        "[workspace]\nresolver = \"3\"\nmembers = [\"{CORE_DIRECTORY}\", \"{CAPTURE_DIRECTORY}\"]\n\n\
         [workspace.package]\nversion = \"0.2.0\"\nedition = \"2024\"\nrust-version = \"1.97.1\"\n\
         license = \"Apache-2.0\"\nrepository = \"https://github.com/pashifika/mado-pilot\"\n"
    )
}

fn member_manifest(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\n\
         rust-version.workspace = true\nlicense.workspace = true\nrepository.workspace = true\n\
         publish = false\n\n[lints]\nworkspace = true\n"
    )
}

/// Writes the root manifest, the toolchain pin, and a manifest for each member.
fn workspace_with_members(label: &str, members: &[(&str, &str)]) -> TempWorkspace {
    let workspace = TempWorkspace::create(label);
    workspace.write("Cargo.toml", &root_manifest());
    workspace.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    for (name, directory) in members {
        workspace.write(&format!("{directory}/Cargo.toml"), &member_manifest(name));
    }
    workspace
}

fn package(root: &Path, name: &str, directory: &str, dependencies: Vec<Value>) -> Value {
    json!({
        "name": name,
        "manifest_path": root.join(directory).join("Cargo.toml"),
        "version": "0.2.0",
        "edition": "2024",
        "rust_version": "1.97.1",
        "license": "Apache-2.0",
        "repository": "https://github.com/pashifika/mado-pilot",
        "publish": [],
        "dependencies": dependencies,
    })
}

/// Builds one dependency entry in Cargo's shape: the real package in `name`, the
/// manifest-visible alias in `rename`, and `path` only for a path source.
fn dependency(name: &str, rename: Option<&str>, path: Option<PathBuf>) -> Value {
    json!({ "name": name, "rename": rename, "kind": null, "path": path })
}

fn read(root: &Path, packages: Vec<Value>) -> Result<WorkspaceObservation, MetadataError> {
    let output = json!({ "workspace_root": root, "packages": packages }).to_string();
    read_metadata_output(output.as_bytes())
}

/// Reads a two-member workspace where capture declares `dependency` on core.
fn observation_with_capture_dependency(
    label: &str,
    dependency: Value,
) -> (TempWorkspace, WorkspaceObservation) {
    let workspace = workspace_with_members(
        label,
        &[(CORE, CORE_DIRECTORY), (CAPTURE, CAPTURE_DIRECTORY)],
    );
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(
            &workspace.root,
            CAPTURE,
            CAPTURE_DIRECTORY,
            vec![dependency],
        ),
    ];
    let observation = read(&workspace.root, packages).expect("the synthetic metadata is readable");
    (workspace, observation)
}

#[test]
fn a_member_path_dependency_is_accepted_as_an_edge() {
    let workspace = workspace_with_members(
        "member-path",
        &[(CORE, CORE_DIRECTORY), (CAPTURE, CAPTURE_DIRECTORY)],
    );
    let core_directory = workspace.root.join(CORE_DIRECTORY);
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(
            &workspace.root,
            CAPTURE,
            CAPTURE_DIRECTORY,
            vec![dependency(CORE, None, Some(core_directory))],
        ),
    ];

    let observation = read(&workspace.root, packages).expect("the synthetic metadata is readable");

    assert_eq!(observation.source_violations, Vec::new());
    assert_eq!(
        observation.graph.edges().to_vec(),
        vec![ObservedEdge::production(CAPTURE, CORE)]
    );
}

#[test]
fn a_non_member_path_dependency_is_rejected() {
    let (_workspace, observation) = observation_with_capture_dependency(
        "non-member-path",
        dependency("vendored-thing", None, Some(PathBuf::from("vendor/thing"))),
    );

    assert_eq!(
        observation.source_violations,
        vec![Violation::NonMemberPathDependency {
            from: CAPTURE.to_owned(),
            dependency: "vendored-thing".to_owned(),
            path: "vendor/thing".to_owned(),
        }]
    );
    assert!(observation.graph.edges().is_empty());
}

#[test]
fn an_in_workspace_path_dependency_that_is_not_a_member_is_rejected() {
    // This reaches the member-set filter rather than the outside-the-root arm: the
    // path strips cleanly against the workspace root, so only the inventory check
    // rejects it. Product code must not enter the build from an undocumented
    // directory inside the repository either.
    let workspace = workspace_with_members(
        "in-workspace-non-member",
        &[(CORE, CORE_DIRECTORY), (CAPTURE, CAPTURE_DIRECTORY)],
    );
    let vendored = workspace.root.join("crates/automation/vendored");
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(
            &workspace.root,
            CAPTURE,
            CAPTURE_DIRECTORY,
            vec![dependency("vendored-thing", None, Some(vendored.clone()))],
        ),
    ];

    let observation = read(&workspace.root, packages).expect("the synthetic metadata is readable");

    assert_eq!(
        observation.source_violations,
        vec![Violation::NonMemberPathDependency {
            from: CAPTURE.to_owned(),
            dependency: "vendored-thing".to_owned(),
            path: vendored.to_string_lossy().into_owned(),
        }]
    );
    assert!(observation.graph.edges().is_empty());
}

#[test]
fn a_member_manifest_outside_the_workspace_root_is_a_tool_failure() {
    let workspace = workspace_with_members("manifest-outside", &[(CORE, CORE_DIRECTORY)]);
    // A sibling of the workspace root, so the path cannot strip against it. Using
    // `root/../outside` would strip successfully, because `strip_prefix` is lexical.
    let outside = workspace
        .root
        .parent()
        .expect("the temporary root has a parent")
        .join("mado-pilot-outside-member");
    let mut stray = package(&workspace.root, CAPTURE, CAPTURE_DIRECTORY, Vec::new());
    stray["manifest_path"] = json!(outside.join("Cargo.toml"));
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        stray,
    ];

    let error =
        read(&workspace.root, packages).expect_err("a manifest outside the root is a tool failure");

    match error {
        MetadataError::ManifestOutsideWorkspace {
            manifest_path,
            workspace_root,
        } => {
            assert_eq!(manifest_path, outside.join("Cargo.toml"));
            assert_eq!(workspace_root, workspace.root);
        }
        other => panic!("expected a manifest outside the workspace, got {other:?}"),
    }
}

#[test]
fn a_registry_dependency_carrying_a_member_name_is_rejected() {
    let (_workspace, observation) =
        observation_with_capture_dependency("same-name-registry", dependency(CORE, None, None));

    assert_eq!(
        observation.source_violations,
        vec![Violation::ShadowedMemberDependency {
            from: CAPTURE.to_owned(),
            to: CORE.to_owned(),
            kind: DependencyKind::Production,
        }]
    );
    assert!(observation.graph.edges().is_empty());
}

#[test]
fn a_registry_dependency_renamed_to_a_member_name_is_rejected() {
    // Cargo reports the real package in `name` and the alias in `rename`. Reading
    // only `name` would skip this dependency as external and let an attacker-owned
    // crates.io package masquerade as an internal contract crate, which `cargo-deny`
    // cannot catch because crates.io is an approved source.
    let (_workspace, observation) = observation_with_capture_dependency(
        "renamed-registry",
        dependency("attacker-package", Some(CORE), None),
    );

    assert_eq!(
        observation.source_violations,
        vec![Violation::ShadowedMemberDependency {
            from: CAPTURE.to_owned(),
            to: CORE.to_owned(),
            kind: DependencyKind::Production,
        }]
    );
    assert!(observation.graph.edges().is_empty());
}

#[test]
fn a_dependency_renamed_to_a_member_name_but_pointing_at_another_member_is_rejected() {
    let workspace = workspace_with_members(
        "renamed-cross-member",
        &[(CORE, CORE_DIRECTORY), (CAPTURE, CAPTURE_DIRECTORY)],
    );
    let capture_directory = workspace.root.join(CAPTURE_DIRECTORY);
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(
            &workspace.root,
            CAPTURE,
            CAPTURE_DIRECTORY,
            vec![dependency(CAPTURE, Some(CORE), Some(capture_directory))],
        ),
    ];

    let observation = read(&workspace.root, packages).expect("the synthetic metadata is readable");

    assert_eq!(
        observation.source_violations,
        vec![Violation::ShadowedMemberDependency {
            from: CAPTURE.to_owned(),
            to: CORE.to_owned(),
            kind: DependencyKind::Production,
        }]
    );
}

#[test]
fn a_member_renamed_to_a_local_alias_is_still_an_edge_to_the_real_package() {
    // Renaming a real member is not masquerading, and the architecture allowlist is
    // written against the real package name.
    let workspace = workspace_with_members(
        "renamed-member",
        &[(CORE, CORE_DIRECTORY), (CAPTURE, CAPTURE_DIRECTORY)],
    );
    let core_directory = workspace.root.join(CORE_DIRECTORY);
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(
            &workspace.root,
            CAPTURE,
            CAPTURE_DIRECTORY,
            vec![dependency(CORE, Some("core-alias"), Some(core_directory))],
        ),
    ];

    let observation = read(&workspace.root, packages).expect("the synthetic metadata is readable");

    assert_eq!(observation.source_violations, Vec::new());
    assert_eq!(
        observation.graph.edges().to_vec(),
        vec![ObservedEdge::production(CAPTURE, CORE)]
    );
}

#[test]
fn cargo_publish_values_decode_into_publishability() {
    let workspace = workspace_with_members("publish-decoding", &[(CORE, CORE_DIRECTORY)]);
    let cases: [(Value, bool); 3] = [
        // `publish = false`
        (json!([]), false),
        // publication unrestricted
        (json!(null), true),
        // publication restricted to a registry, which is still publishable
        (json!(["madopilot-internal"]), true),
    ];

    for (publish, publishable) in cases {
        let mut package = package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new());
        package["publish"] = publish.clone();
        let observation =
            read(&workspace.root, vec![package]).expect("the synthetic metadata is readable");

        assert_eq!(
            observation.members[0].publishable, publishable,
            "`publish = {publish}` must decode to publishable {publishable}"
        );
    }
}

#[test]
fn a_member_manifest_that_cannot_be_read_is_a_tool_failure() {
    // Only core's manifest is written, so capture's manifest is missing.
    let workspace = workspace_with_members("unreadable-manifest", &[(CORE, CORE_DIRECTORY)]);
    let packages = vec![
        package(&workspace.root, CORE, CORE_DIRECTORY, Vec::new()),
        package(&workspace.root, CAPTURE, CAPTURE_DIRECTORY, Vec::new()),
    ];

    let error = read(&workspace.root, packages).expect_err("a missing manifest is a tool failure");

    match error {
        MetadataError::ManifestUnreadable { manifest_path, .. } => assert!(
            manifest_path.ends_with(Path::new(CAPTURE_DIRECTORY).join("Cargo.toml")),
            "{manifest_path:?}"
        ),
        other => panic!("expected an unreadable manifest, got {other:?}"),
    }
}

#[test]
fn a_workspace_without_reserved_directories_reports_nothing() {
    let workspace = workspace_with_members("no-reserved", &[(CORE, CORE_DIRECTORY)]);

    assert_eq!(deferred_directory_violations(&workspace.root), Vec::new());
}

#[test]
fn an_empty_reserved_adapter_directory_is_reported() {
    let workspace = workspace_with_members("empty-reserved", &[(CORE, CORE_DIRECTORY)]);
    fs::create_dir_all(workspace.path("crates/platform/adb"))
        .expect("the reserved directory can be created");

    let violations = deferred_directory_violations(&workspace.root);

    assert_eq!(reserved_directories(&violations), ["crates/platform/adb"]);
}

#[test]
fn a_dangling_reserved_adapter_symlink_is_reported() {
    let workspace = workspace_with_members("dangling-reserved", &[(CORE, CORE_DIRECTORY)]);
    let link = workspace.path("crates/platform/adb");
    fs::create_dir_all(link.parent().expect("the link has a parent directory"))
        .expect("the reserved parent directory can be created");

    // `Path::exists` follows symlinks and reports `false` for a broken one, so a
    // tracked broken link could otherwise occupy a reserved adapter path unnoticed.
    if symlink_directory(&workspace.path("crates/platform/absent-target"), &link).is_err() {
        // Creating a symlink needs a privilege that is not always available, most
        // notably on Windows outside Developer Mode. The rule is platform
        // independent, so skipping here is honest rather than a silent pass.
        eprintln!("skipped: this platform did not permit creating a symlink");
        return;
    }

    let violations = deferred_directory_violations(&workspace.root);

    assert_eq!(reserved_directories(&violations), ["crates/platform/adb"]);
}

#[test]
fn a_source_violation_reaches_the_command_line_diagnostic() {
    // The one case that runs Cargo, because exit status and diagnostics are the
    // contract the command-line interface owns. The path dependency points outside
    // the workspace root, so Cargo does not adopt it as a member.
    let workspace = TempWorkspace::create("cli-source-violation");
    workspace.write(
        "workspace/Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"member\"]\n",
    );
    workspace.write(
        "workspace/member/Cargo.toml",
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\noutside = { path = \"../../outside\" }\n",
    );
    workspace.write("workspace/member/src/lib.rs", "");
    workspace.write(
        "outside/Cargo.toml",
        "[package]\nname = \"outside\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         [workspace]\n",
    );
    workspace.write("outside/src/lib.rs", "");

    let output = Command::new(env!("CARGO_BIN_EXE_mado-pilot-dependency-check"))
        .arg("--manifest-path")
        .arg(workspace.path("workspace/Cargo.toml"))
        .output()
        .expect("the checker binary can be run");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("declares a path dependency `outside`"),
        "{stderr}"
    );
    assert!(
        stderr.contains("which is not a workspace member"),
        "{stderr}"
    );
    assert!(stderr.contains("architecture check failed"), "{stderr}");
}

fn reserved_directories(violations: &[Violation]) -> Vec<&str> {
    violations
        .iter()
        .filter_map(|violation| match violation {
            Violation::DeferredDirectory { directory, .. } => Some(directory.as_str()),
            _ => None,
        })
        .collect()
}

#[cfg(unix)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_directory(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_directory(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("symlinks are unavailable here"))
}
