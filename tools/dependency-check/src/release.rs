//! Deterministic committed source-release validation and its Git-tree adapter.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Product version whose source-release scope this module validates.
pub const RELEASE_VERSION: &str = "0.4.0";
/// Canonical release body relative to the workspace root.
pub const RELEASE_NOTES_FILE: &str = "docs/releases/v0.4.0.md";
/// Development-consumer CMake project relative to the workspace root.
pub const CMAKE_PROJECT_FILE: &str = "crates/bindings/capi/CMakeLists.txt";
/// Public C header relative to the workspace root.
pub const C_HEADER_FILE: &str = "crates/bindings/capi/include/madopilot/madopilot.h";
/// Public C++ header relative to the workspace root.
pub const CPP_HEADER_FILE: &str = "crates/bindings/capi/include/madopilot/madopilot.hpp";
const CMAKE_VERSION_DECLARATION: &str = "VERSION 0.4.0";
const PUBLIC_HEADER_DIR: &str = "crates/bindings/capi/include/madopilot";

/// Git subtree identities frozen for the v0.4.0 source-release boundary.
pub const REQUIRED_TREE_IDENTITIES: [(&str, &str); 5] = [
    (".cargo", "46f2deca6a18229279e14433738418c4df501be9"),
    (".github", "9ea02fb5851e6ab15818fc3de36456f6de074d94"),
    ("crates", "509f827d161243d574adf5b24ecf856a2143525b"),
    ("docs", "a4b22fb0a9427a973a3bcfb445615232ac445f8e"),
    ("fixtures", "fbfc3e6eabd95b4bbae4926929f9ad593a170d99"),
];

/// Git blob identities that control provider archive construction.
pub const REQUIRED_BLOB_IDENTITIES: [(&str, &str); 1] =
    [(".gitattributes", "8707e22ae84c17d12e411ab5e769da0a274ad271")];

const PINNED_TREE_PREFIXES: [&str; 5] = [".cargo/", ".github/", "crates/", "docs/", "fixtures/"];

const REQUIRED_RELEASE_FACTS: [(&str, &str); 12] = [
    ("release identity", "# MadoPilot v0.4.0"),
    (
        "canonical release body",
        "This file is the canonical release body.",
    ),
    ("replay/OpenCV watcher", "deterministic replay/OpenCV"),
    ("Windows WGC watcher", "Windows Graphics Capture (WGC)"),
    ("macOS ScreenCaptureKit watcher", "ScreenCaptureKit"),
    ("Windows release target", "x86_64-pc-windows-msvc"),
    ("macOS release target", "aarch64-apple-darwin"),
    ("unchanged ABI 1.5", "C ABI 1.5 remains unchanged."),
    (
        "absent C/C++ watcher",
        "The C and C++ watcher APIs are unavailable.",
    ),
    (
        "source-only publication",
        "This source-only release publishes the permanent annotated tag, this tracked release body, and provider-generated source archives only.",
    ),
    (
        "unavailable artifact inventory",
        "Crates.io packages, prebuilt or static libraries, installers, package-manager artifacts, CMake install/export metadata, pkg-config metadata, ABI-major decorated libraries, and bundled OpenCV, ONNX Runtime, OCR models, CUDA, or cuDNN are not provided.",
    ),
    (
        "privacy exclusions",
        "Ordinary diagnostics and release evidence exclude captured pixels, recognized text, caller template/model identities, input payloads, credentials, raw native identifiers, unrelated process/window inventories, and local paths.",
    ),
];

const FOREIGN_WATCHER_TOKENS: [&str; 7] = [
    "madopilot_template_watch",
    "madopilot_template_query",
    "start_template_watch",
    "templatewatch",
    "templatequery",
    "watch_template",
    "watch_query",
];

const ALLOWED_ADD_LIBRARY_COMMANDS: [&str; 3] = [
    "add_library(MadoPilot::C SHARED IMPORTED GLOBAL)",
    "add_library(madopilot_cpp INTERFACE)",
    "add_library(MadoPilot::Cpp ALIAS madopilot_cpp)",
];

const FORBIDDEN_SEGMENTS: [(&str, &str); 11] = [
    ("rasen", "nested planning repository"),
    (".rasen", "local planning ephemera"),
    ("local_docs", "private local documentation"),
    (".claude", "contributor-local agent configuration"),
    (".agents", "contributor-local agent configuration"),
    ("target", "generated Cargo output"),
    ("debug", "generated build output"),
    (".idea", "contributor-local IDE state"),
    (".qualification", "private qualification output"),
    ("qualification-output", "private qualification output"),
    ("qualification_artifacts", "private qualification output"),
];

const PAYLOAD_EXTENSIONS: [&str; 33] = [
    "a", "bin", "bmp", "bz2", "deb", "dll", "dmg", "dylib", "exe", "gif", "gz", "ico", "jpeg",
    "jpg", "key", "lib", "msi", "onnx", "p12", "pdb", "pem", "pfx", "pkg", "png", "rgba", "rpm",
    "so", "tar", "tgz", "webp", "xz", "zip", "zst",
];

const BINARY_MAGICS: [&[u8]; 23] = [
    b"PK\x03\x04",
    b"PK\x05\x06",
    b"PK\x07\x08",
    b"\x7fELF",
    b"MZ",
    b"!<arch>\n",
    b"\0asm",
    b"\xfe\xed\xfa\xce",
    b"\xce\xfa\xed\xfe",
    b"\xfe\xed\xfa\xcf",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
    b"\xbe\xba\xfe\xca",
    b"\xca\xfe\xba\xbf",
    b"\xbf\xba\xfe\xca",
    b"\x1f\x8b",
    b"BZh",
    b"\xfd7zXZ\0",
    b"\x28\xb5\x2f\xfd",
    b"7z\xbc\xaf\x27\x1c",
    b"Rar!\x1a\x07",
    b"%PDF-",
    b"SQLite format 3\0",
];

const ALLOWED_G014_ZIPS: [&str; 24] = [
    "fixtures/assets/g-014/adversarial/bomb-compression-ratio.zip",
    "fixtures/assets/g-014/adversarial/bomb-entry-count-declared.zip",
    "fixtures/assets/g-014/adversarial/bomb-entry-uncompressed-declared.zip",
    "fixtures/assets/g-014/adversarial/bomb-total-uncompressed-declared.zip",
    "fixtures/assets/g-014/adversarial/bomb-understated-declaration.zip",
    "fixtures/assets/g-014/adversarial/entry-character-device.zip",
    "fixtures/assets/g-014/adversarial/entry-directory-name-collision.zip",
    "fixtures/assets/g-014/adversarial/entry-fifo.zip",
    "fixtures/assets/g-014/adversarial/entry-symlink.zip",
    "fixtures/assets/g-014/adversarial/hash-mismatch.zip",
    "fixtures/assets/g-014/adversarial/manifest-malformed.zip",
    "fixtures/assets/g-014/adversarial/manifest-missing.zip",
    "fixtures/assets/g-014/adversarial/manifest-oversize.zip",
    "fixtures/assets/g-014/adversarial/manifest-unsupported-schema.zip",
    "fixtures/assets/g-014/adversarial/path-absolute-drive.zip",
    "fixtures/assets/g-014/adversarial/path-absolute-posix.zip",
    "fixtures/assets/g-014/adversarial/path-backslash-separator.zip",
    "fixtures/assets/g-014/adversarial/path-duplicate-normalized.zip",
    "fixtures/assets/g-014/adversarial/path-embedded-nul.zip",
    "fixtures/assets/g-014/adversarial/path-non-utf8.zip",
    "fixtures/assets/g-014/adversarial/path-traversal-inner.zip",
    "fixtures/assets/g-014/adversarial/path-traversal.zip",
    "fixtures/assets/g-014/adversarial/path-unc-root.zip",
    "fixtures/assets/g-014/valid/valid-tiny.zip",
];

const ALLOWED_ONNX_FIXTURES: [(&str, &str); 2] = [
    (
        "fixtures/assets/ocr-public-surface/models/detector.onnx",
        "cc6723e5145af0e74428c9056f84709dfd06661c",
    ),
    (
        "fixtures/assets/ocr-public-surface/models/recognizer.onnx",
        "b7c572ac50ba66d44439245d16926ef4a62baed3",
    ),
];

const ALLOWED_EXECUTABLES: [(&str, &str); 3] = [
    (
        "docs/evidence/g-004/evaluate.py",
        "d5fae53a3e614aacbb608523dc28603a6c8a3995",
    ),
    (
        "docs/evidence/g-004/validate.py",
        "7fc4edaee10ead4d9a8cbc6f9f5e613aa1386ffd",
    ),
    (
        "fixtures/ocr/g-004/generate.py",
        "78e78b99bf06643c08f5d214dba3f387af51d789",
    ),
];

const ALLOWED_PINNED_FIXTURE_EXTENSIONS: [&str; 3] = ["bin", "png", "rgba"];

const ALLOWED_EXTENSIONLESS_FILES: [&str; 6] = [
    "LICENSE",
    "fixtures/assets/g-014/SHA256SUMS",
    "fixtures/assets/phase1-slice/SHA256SUMS",
    "fixtures/capture/replay-basic/SHA256SUMS",
    "fixtures/change-detection/g-005/SHA256SUMS",
    "fixtures/ocr/g-004/SHA256SUMS",
];

const ALLOWED_SYMLINK: (&str, &str, &str) = (
    "AGENTS.md",
    "CLAUDE.md",
    "681311eb9cf453d0faddf3aacaec7357e97ba8e9",
);

/// One blob or gitlink emitted by the candidate Git tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedEntry {
    /// Git tree mode, such as `100644`, `100755`, or `120000`.
    pub mode: String,
    /// Git object identity for the entry.
    pub object_id: String,
    /// Symlink target bytes decoded as UTF-8; absent for non-symlinks.
    pub symlink_target: Option<String>,
    /// Whether an unpinned regular blob passes text-safety validation.
    pub safe_text: Option<bool>,
}

/// Release inputs observed from the committed candidate tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseScopeObservation {
    /// Candidate-tree blobs and gitlinks keyed by workspace-relative path.
    pub tracked_entries: BTreeMap<String, TrackedEntry>,
    /// Frozen release-relevant subtree identities.
    pub tree_oids: BTreeMap<String, String>,
    /// Canonical release body text.
    pub release_notes: String,
    /// Development-consumer CMake project text.
    pub cmake_project: String,
    /// Public C header text.
    pub c_header: String,
    /// Public C++ header text.
    pub cpp_header: String,
}

/// A release-scope rule violated by an observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseViolation {
    /// The canonical release body is not present in the candidate tree.
    UntrackedReleaseNotes,
    /// A required public release fact is absent from the canonical body.
    MissingReleaseFact {
        /// Stable description of the missing fact.
        fact: &'static str,
    },
    /// A frozen release-relevant subtree changed.
    UnexpectedTreeIdentity {
        /// Workspace-relative subtree whose identity changed.
        tree: &'static str,
    },
    /// The development-consumer project does not declare the product version.
    UnexpectedCmakeVersion,
    /// The CMake project contains an unavailable packaging command or kind.
    ForbiddenCmakeSurface {
        /// Command or library kind that appeared.
        surface: &'static str,
    },
    /// A foreign-language public header exposes a watcher token.
    ForeignWatcherSurface {
        /// Header containing the token.
        header: &'static str,
        /// Token that appeared.
        token: &'static str,
    },
    /// A tracked path, mode, target, or payload violates the release boundary.
    ForbiddenReleaseInput {
        /// Git-tracked path that violates the rule.
        path: String,
        /// Stable reason for exclusion.
        reason: &'static str,
    },
}

impl fmt::Display for ReleaseViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UntrackedReleaseNotes => write!(
                formatter,
                "canonical release body `{RELEASE_NOTES_FILE}` is absent from the candidate tree"
            ),
            Self::MissingReleaseFact { fact } => {
                write!(
                    formatter,
                    "canonical release body omits required fact: {fact}"
                )
            }
            Self::UnexpectedTreeIdentity { tree } => {
                write!(formatter, "release-relevant Git subtree `{tree}` changed")
            }
            Self::UnexpectedCmakeVersion => write!(
                formatter,
                "`{CMAKE_PROJECT_FILE}` must declare project version {RELEASE_VERSION}"
            ),
            Self::ForbiddenCmakeSurface { surface } => write!(
                formatter,
                "`{CMAKE_PROJECT_FILE}` exposes unavailable packaging surface `{surface}`"
            ),
            Self::ForeignWatcherSurface { header, token } => write!(
                formatter,
                "public header `{header}` exposes unsupported watcher token `{token}`"
            ),
            Self::ForbiddenReleaseInput { path, reason } => {
                write!(
                    formatter,
                    "tracked release input `{path}` is forbidden: {reason}"
                )
            }
        }
    }
}

/// A failure that prevented release inputs from being inspected.
#[derive(Debug)]
pub enum ReleaseScopeError {
    /// Git could not be started.
    GitSpawn(std::io::Error),
    /// Git rejected a candidate-tree query.
    GitFailed {
        /// Git process status code when available.
        status: Option<i32>,
        /// Bounded diagnostic text emitted by Git.
        stderr: String,
    },
    /// Git returned a path that is not UTF-8.
    NonUtf8TrackedPath,
    /// A tracked text blob or symlink target is not UTF-8.
    NonUtf8TrackedContent {
        /// Workspace-relative path whose blob is not UTF-8.
        path: String,
    },
    /// Git returned a malformed tree record.
    MalformedGitOutput {
        /// Stable operation name whose output was malformed.
        operation: &'static str,
    },
    /// A required source file is absent from the candidate tree.
    MissingTrackedInput {
        /// Required workspace-relative source path.
        path: &'static str,
    },
}

impl fmt::Display for ReleaseScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitSpawn(error) => write!(formatter, "could not start Git: {error}"),
            Self::GitFailed { status, stderr } => write!(
                formatter,
                "Git candidate-tree query failed with status {}: {}",
                status.map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
                stderr.trim()
            ),
            Self::NonUtf8TrackedPath => {
                formatter.write_str("Git candidate tree contains a non-UTF-8 path")
            }
            Self::NonUtf8TrackedContent { path } => {
                write!(formatter, "tracked release input `{path}` is not UTF-8")
            }
            Self::MalformedGitOutput { operation } => {
                write!(formatter, "Git returned malformed output for `{operation}`")
            }
            Self::MissingTrackedInput { path } => {
                write!(
                    formatter,
                    "candidate tree omits required release input `{path}`"
                )
            }
        }
    }
}

impl Error for ReleaseScopeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GitSpawn(error) => Some(error),
            Self::GitFailed { .. }
            | Self::NonUtf8TrackedPath
            | Self::NonUtf8TrackedContent { .. }
            | Self::MalformedGitOutput { .. }
            | Self::MissingTrackedInput { .. } => None,
        }
    }
}

/// Reads the release-scope observation from the committed candidate tree.
///
/// # Errors
///
/// Returns [`ReleaseScopeError`] when Git cannot enumerate `HEAD`, a tracked
/// path or required text blob is not UTF-8, or a required input is absent.
pub fn read_workspace(workspace_root: &Path) -> Result<ReleaseScopeObservation, ReleaseScopeError> {
    let output = git_output(workspace_root, &["ls-tree", "-rz", "--full-tree", "HEAD"])?;
    let tracked_output =
        std::str::from_utf8(&output).map_err(|_| ReleaseScopeError::NonUtf8TrackedPath)?;
    let mut tracked_entries = BTreeMap::new();

    for record in tracked_output
        .split('\0')
        .filter(|record| !record.is_empty())
    {
        let (metadata, path) =
            record
                .split_once('\t')
                .ok_or(ReleaseScopeError::MalformedGitOutput {
                    operation: "ls-tree",
                })?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().ok_or(ReleaseScopeError::MalformedGitOutput {
            operation: "ls-tree",
        })?;
        let kind = fields.next().ok_or(ReleaseScopeError::MalformedGitOutput {
            operation: "ls-tree",
        })?;
        let object_id = fields.next().ok_or(ReleaseScopeError::MalformedGitOutput {
            operation: "ls-tree",
        })?;
        if fields.next().is_some() || !matches!(kind, "blob" | "commit") {
            return Err(ReleaseScopeError::MalformedGitOutput {
                operation: "ls-tree",
            });
        }

        let (symlink_target, safe_text) = match mode {
            "120000" => (Some(read_blob_text(workspace_root, object_id, path)?), None),
            "100644" | "100755" if !path_in_pinned_tree(path) => {
                let bytes = read_blob_bytes(workspace_root, object_id)?;
                (None, Some(safe_text_blob(&bytes)))
            }
            _ => (None, None),
        };
        let previous = tracked_entries.insert(
            path.to_owned(),
            TrackedEntry {
                mode: mode.to_owned(),
                object_id: object_id.to_owned(),
                symlink_target,
                safe_text,
            },
        );
        if previous.is_some() {
            return Err(ReleaseScopeError::MalformedGitOutput {
                operation: "ls-tree",
            });
        }
    }

    let release_notes = read_required_blob(workspace_root, &tracked_entries, RELEASE_NOTES_FILE)?;
    let cmake_project = read_required_blob(workspace_root, &tracked_entries, CMAKE_PROJECT_FILE)?;
    let c_header = read_required_blob(workspace_root, &tracked_entries, C_HEADER_FILE)?;
    let cpp_header = read_required_blob(workspace_root, &tracked_entries, CPP_HEADER_FILE)?;
    let tree_oids = REQUIRED_TREE_IDENTITIES
        .iter()
        .map(|(path, _)| {
            read_tree_oid(workspace_root, path).map(|object_id| ((*path).to_owned(), object_id))
        })
        .collect::<Result<_, _>>()?;

    Ok(ReleaseScopeObservation {
        tracked_entries,
        tree_oids,
        release_notes,
        cmake_project,
        c_header,
        cpp_header,
    })
}

/// Validates one release-scope observation in deterministic rule order.
#[must_use]
pub fn validate(observation: &ReleaseScopeObservation) -> Vec<ReleaseViolation> {
    let mut violations = Vec::new();

    if !observation.tracked_entries.contains_key(RELEASE_NOTES_FILE) {
        violations.push(ReleaseViolation::UntrackedReleaseNotes);
    }

    for (fact, needle) in REQUIRED_RELEASE_FACTS {
        if !observation.release_notes.contains(needle) {
            violations.push(ReleaseViolation::MissingReleaseFact { fact });
        }
    }

    for (tree, expected) in REQUIRED_TREE_IDENTITIES {
        if observation.tree_oids.get(tree).map(String::as_str) != Some(expected) {
            violations.push(ReleaseViolation::UnexpectedTreeIdentity { tree });
        }
    }

    for (path, expected) in REQUIRED_BLOB_IDENTITIES {
        let observed = observation
            .tracked_entries
            .get(path)
            .map(|entry| entry.object_id.as_str());
        if observed != Some(expected) {
            violations.push(ReleaseViolation::ForbiddenReleaseInput {
                path: path.to_owned(),
                reason: "required provider-archive control changed or is absent",
            });
        }
    }

    if !observation
        .cmake_project
        .lines()
        .any(|line| line.trim() == CMAKE_VERSION_DECLARATION)
    {
        violations.push(ReleaseViolation::UnexpectedCmakeVersion);
    }

    for command in ["install", "export", "include", "add_subdirectory"] {
        if has_cmake_command(&observation.cmake_project, command) {
            violations.push(ReleaseViolation::ForbiddenCmakeSurface { surface: command });
        }
    }
    if cmake_command_lines(&observation.cmake_project, "add_library").any(|code| {
        !ALLOWED_ADD_LIBRARY_COMMANDS
            .iter()
            .any(|allowed| code.eq_ignore_ascii_case(allowed))
    }) {
        violations.push(ReleaseViolation::ForbiddenCmakeSurface {
            surface: "add_library",
        });
    }
    if has_cmake_token(&observation.cmake_project, "STATIC") {
        violations.push(ReleaseViolation::ForbiddenCmakeSurface { surface: "STATIC" });
    }

    for (header, text) in [
        (C_HEADER_FILE, observation.c_header.as_str()),
        (CPP_HEADER_FILE, observation.cpp_header.as_str()),
    ] {
        let text = text.to_ascii_lowercase();
        for token in FOREIGN_WATCHER_TOKENS {
            if text.contains(token) {
                violations.push(ReleaseViolation::ForeignWatcherSurface { header, token });
            }
        }
    }

    for (path, entry) in &observation.tracked_entries {
        let reason = forbidden_tracked_entry(path, entry, &observation.tracked_entries)
            .or_else(|| forbidden_release_input(path, entry));
        if let Some(reason) = reason {
            violations.push(ReleaseViolation::ForbiddenReleaseInput {
                path: path.clone(),
                reason,
            });
        }
    }

    violations
}

fn git_program() -> PathBuf {
    std::env::var_os("GIT").map_or_else(|| PathBuf::from("git"), PathBuf::from)
}

fn git_output(workspace_root: &Path, arguments: &[&str]) -> Result<Vec<u8>, ReleaseScopeError> {
    let output = Command::new(git_program())
        .arg("-C")
        .arg(workspace_root)
        .args(arguments)
        .output()
        .map_err(ReleaseScopeError::GitSpawn)?;
    if !output.status.success() {
        return Err(ReleaseScopeError::GitFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(output.stdout)
}

fn read_required_blob(
    workspace_root: &Path,
    tracked_entries: &BTreeMap<String, TrackedEntry>,
    path: &'static str,
) -> Result<String, ReleaseScopeError> {
    let entry = tracked_entries
        .get(path)
        .ok_or(ReleaseScopeError::MissingTrackedInput { path })?;
    read_blob_text(workspace_root, &entry.object_id, path)
}

fn read_blob_text(
    workspace_root: &Path,
    object_id: &str,
    path: &str,
) -> Result<String, ReleaseScopeError> {
    let bytes = read_blob_bytes(workspace_root, object_id)?;
    String::from_utf8(bytes).map_err(|_| ReleaseScopeError::NonUtf8TrackedContent {
        path: path.to_owned(),
    })
}

fn read_blob_bytes(workspace_root: &Path, object_id: &str) -> Result<Vec<u8>, ReleaseScopeError> {
    git_output(workspace_root, &["cat-file", "blob", object_id])
}

fn safe_text_blob(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let has_forbidden_control = text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'));
    !has_forbidden_control
        && !BINARY_MAGICS.iter().any(|magic| bytes.starts_with(magic))
        && bytes.get(257..262) != Some(b"ustar")
}

fn read_tree_oid(workspace_root: &Path, path: &str) -> Result<String, ReleaseScopeError> {
    let revision = format!("HEAD:{path}");
    let output = git_output(workspace_root, &["rev-parse", "--verify", &revision])?;
    let object_id =
        std::str::from_utf8(&output).map_err(|_| ReleaseScopeError::MalformedGitOutput {
            operation: "rev-parse",
        })?;
    let object_id = object_id.trim();
    if object_id.is_empty() {
        return Err(ReleaseScopeError::MalformedGitOutput {
            operation: "rev-parse",
        });
    }
    Ok(object_id.to_owned())
}

fn has_cmake_command(text: &str, command: &str) -> bool {
    cmake_command_lines(text, command).next().is_some()
}

fn cmake_command_lines<'a>(text: &'a str, command: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    text.lines().filter_map(move |line| {
        let code = line.split_once('#').map_or(line, |(code, _)| code).trim();
        let prefix = code.get(..command.len())?;
        (prefix.eq_ignore_ascii_case(command)
            && code[command.len()..].trim_start().starts_with('('))
        .then_some(code)
    })
}

fn has_cmake_token(text: &str, token: &str) -> bool {
    text.lines().any(|line| {
        let code = line.split_once('#').map_or(line, |(code, _)| code);
        code.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .any(|word| word.eq_ignore_ascii_case(token))
    })
}

fn forbidden_tracked_entry(
    path: &str,
    entry: &TrackedEntry,
    tracked_entries: &BTreeMap<String, TrackedEntry>,
) -> Option<&'static str> {
    match (entry.mode.as_str(), entry.symlink_target.as_deref()) {
        ("100644", None) => {
            if !path_in_pinned_tree(path) && entry.safe_text != Some(true) {
                return Some("unpinned regular file failed text-safety validation");
            }
            let file_name = path.rsplit('/').next().unwrap_or(path);
            let has_extension = file_name.rsplit_once('.').is_some();
            if !has_extension && !ALLOWED_EXTENSIONLESS_FILES.contains(&path) {
                Some("unapproved extensionless file")
            } else {
                None
            }
        }
        ("100755", None) if allowed_executable(path, &entry.object_id) => None,
        ("100755", None) => Some("unapproved executable file or content"),
        ("120000", Some(target))
            if path == ALLOWED_SYMLINK.0
                && target == ALLOWED_SYMLINK.1
                && entry.object_id == ALLOWED_SYMLINK.2
                && tracked_entries.contains_key(target) =>
        {
            None
        }
        ("120000", _) => Some("unapproved symlink or symlink target"),
        ("160000", _) => Some("Gitlink or nested repository"),
        _ => Some("unsupported Git tree mode"),
    }
}

fn forbidden_release_input(path: &str, entry: &TrackedEntry) -> Option<&'static str> {
    for segment in path.split('/') {
        if let Some((_, reason)) = FORBIDDEN_SEGMENTS
            .iter()
            .find(|(candidate, _)| *candidate == segment)
        {
            return Some(reason);
        }
    }

    if path
        .strip_prefix(PUBLIC_HEADER_DIR)
        .is_some_and(|suffix| suffix.starts_with('/'))
        && path != C_HEADER_FILE
        && path != CPP_HEADER_FILE
    {
        return Some("unapproved public foreign-language header");
    }

    let file_name = path.rsplit('/').next().unwrap_or(path);
    if file_name == ".gitattributes"
        && (path != ".gitattributes" || entry.object_id != REQUIRED_BLOB_IDENTITIES[0].1)
    {
        return Some("unapproved archive attribute file or content");
    }
    if file_name == ".DS_Store" || file_name.ends_with(".rs.bk") {
        return Some("generated editor or operating-system file");
    }
    if file_name == ".env" || file_name.starts_with(".env.") {
        return Some("private environment file");
    }
    if path
        .split('/')
        .any(|segment| segment.starts_with("mutants.out"))
    {
        return Some("generated mutation-test output");
    }

    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
    let is_payload = extension.is_some_and(|extension| {
        PAYLOAD_EXTENSIONS
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    });
    if is_payload && !allowed_fixture_payload(path, &entry.object_id) {
        return Some("unapproved archive, model, credential, or native binary payload");
    }

    None
}

fn allowed_fixture_payload(path: &str, object_id: &str) -> bool {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
    let pinned_fixture = path.starts_with("fixtures/")
        && extension.is_some_and(|extension| {
            ALLOWED_PINNED_FIXTURE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        });
    pinned_fixture
        || ALLOWED_G014_ZIPS.contains(&path)
        || ALLOWED_ONNX_FIXTURES
            .iter()
            .any(|(candidate, expected)| path == *candidate && object_id == *expected)
}

fn allowed_executable(path: &str, object_id: &str) -> bool {
    ALLOWED_EXECUTABLES
        .iter()
        .any(|(candidate, expected)| path == *candidate && object_id == *expected)
}

fn path_in_pinned_tree(path: &str) -> bool {
    PINNED_TREE_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::safe_text_blob;

    #[test]
    fn text_safety_rejects_valid_utf8_binary_controls_and_magic() {
        assert!(!safe_text_blob(b"private\0payload"));
        assert!(!safe_text_blob(b"MZprintable-payload"));
        assert!(!safe_text_blob(b"\0asm\x01\0\0\0"));

        let mut tar = vec![b' '; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert!(!safe_text_blob(&tar));
    }

    #[test]
    fn text_safety_accepts_source_text_controls_only() {
        assert!(safe_text_blob("source\ttext\n魔導士\r\n".as_bytes()));
    }
}
