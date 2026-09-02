//! Deterministic source-release scope validation and its Git/filesystem adapter.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
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
        "Ordinary diagnostics and release evidence exclude captured pixels",
    ),
];

const FOREIGN_WATCHER_TOKENS: [&str; 5] = [
    "madopilot_template_watch",
    "madopilot_template_query",
    "start_template_watch",
    "TemplateWatch",
    "TemplateQuery",
];

const FORBIDDEN_PREFIXES: [(&str, &str); 10] = [
    ("rasen/", "nested planning repository"),
    (".rasen/", "local planning ephemera"),
    ("local_docs/", "private local documentation"),
    (".claude/", "contributor-local agent configuration"),
    (".agents/", "contributor-local agent configuration"),
    ("target/", "generated Cargo output"),
    ("debug/", "generated build output"),
    (".qualification/", "private qualification output"),
    ("qualification-output/", "private qualification output"),
    ("qualification_artifacts/", "private qualification output"),
];

const PAYLOAD_EXTENSIONS: [&str; 19] = [
    "a", "bz2", "deb", "dll", "dmg", "exe", "gz", "lib", "msi", "onnx", "pdb", "pkg", "rpm", "so",
    "tar", "tgz", "xz", "zip", "zst",
];

const ALLOWED_ONNX_FIXTURES: [&str; 2] = [
    "fixtures/assets/ocr-public-surface/models/detector.onnx",
    "fixtures/assets/ocr-public-surface/models/recognizer.onnx",
];

/// Release inputs observed from the current checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseScopeObservation {
    /// Git-tracked workspace-relative paths.
    pub tracked_paths: BTreeSet<String>,
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
    /// The canonical release body is not present in Git's index.
    UntrackedReleaseNotes,
    /// A required public release fact is absent from the canonical body.
    MissingReleaseFact {
        /// Stable description of the missing fact.
        fact: &'static str,
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
    /// A tracked path is private, generated, or an unapproved binary payload.
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
                "canonical release body `{RELEASE_NOTES_FILE}` is not Git-tracked"
            ),
            Self::MissingReleaseFact { fact } => {
                write!(
                    formatter,
                    "canonical release body omits required fact: {fact}"
                )
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
    /// Git rejected the tracked-path query.
    GitFailed {
        /// Git process status code when available.
        status: Option<i32>,
        /// Bounded diagnostic text emitted by Git.
        stderr: String,
    },
    /// Git returned a tracked path that is not UTF-8.
    NonUtf8TrackedPath,
    /// A required tracked source file could not be read.
    ReadFile {
        /// Workspace-relative source path.
        path: &'static str,
        /// Filesystem failure.
        source: std::io::Error,
    },
}

impl fmt::Display for ReleaseScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitSpawn(error) => write!(formatter, "could not start Git: {error}"),
            Self::GitFailed { status, stderr } => write!(
                formatter,
                "Git tracked-path query failed with status {}: {}",
                status.map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
                stderr.trim()
            ),
            Self::NonUtf8TrackedPath => {
                formatter.write_str("Git tracked-path output contains a non-UTF-8 path")
            }
            Self::ReadFile { path, source } => {
                write!(formatter, "could not read release input `{path}`: {source}")
            }
        }
    }
}

impl Error for ReleaseScopeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GitSpawn(error) | Self::ReadFile { source: error, .. } => Some(error),
            Self::GitFailed { .. } | Self::NonUtf8TrackedPath => None,
        }
    }
}

/// Reads the release-scope observation from a Git checkout.
///
/// # Errors
///
/// Returns [`ReleaseScopeError`] when Git cannot enumerate the index, a tracked
/// path is not UTF-8, or one of the required release inputs cannot be read.
pub fn read_workspace(workspace_root: &Path) -> Result<ReleaseScopeObservation, ReleaseScopeError> {
    let output = Command::new(git_program())
        .arg("-C")
        .arg(workspace_root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(ReleaseScopeError::GitSpawn)?;

    if !output.status.success() {
        return Err(ReleaseScopeError::GitFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let tracked_output =
        std::str::from_utf8(&output.stdout).map_err(|_| ReleaseScopeError::NonUtf8TrackedPath)?;
    let tracked_paths = tracked_output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect();

    Ok(ReleaseScopeObservation {
        tracked_paths,
        release_notes: read_required(workspace_root, RELEASE_NOTES_FILE)?,
        cmake_project: read_required(workspace_root, CMAKE_PROJECT_FILE)?,
        c_header: read_required(workspace_root, C_HEADER_FILE)?,
        cpp_header: read_required(workspace_root, CPP_HEADER_FILE)?,
    })
}

/// Validates one release-scope observation in deterministic rule order.
#[must_use]
pub fn validate(observation: &ReleaseScopeObservation) -> Vec<ReleaseViolation> {
    let mut violations = Vec::new();

    if !observation.tracked_paths.contains(RELEASE_NOTES_FILE) {
        violations.push(ReleaseViolation::UntrackedReleaseNotes);
    }

    for (fact, needle) in REQUIRED_RELEASE_FACTS {
        if !observation.release_notes.contains(needle) {
            violations.push(ReleaseViolation::MissingReleaseFact { fact });
        }
    }

    if !observation
        .cmake_project
        .lines()
        .any(|line| line.trim() == CMAKE_VERSION_DECLARATION)
    {
        violations.push(ReleaseViolation::UnexpectedCmakeVersion);
    }

    for command in ["install", "export"] {
        if has_cmake_command(&observation.cmake_project, command) {
            violations.push(ReleaseViolation::ForbiddenCmakeSurface { surface: command });
        }
    }
    if has_cmake_token(&observation.cmake_project, "STATIC") {
        violations.push(ReleaseViolation::ForbiddenCmakeSurface { surface: "STATIC" });
    }

    for (header, text) in [
        (C_HEADER_FILE, observation.c_header.as_str()),
        (CPP_HEADER_FILE, observation.cpp_header.as_str()),
    ] {
        for token in FOREIGN_WATCHER_TOKENS {
            if text.contains(token) {
                violations.push(ReleaseViolation::ForeignWatcherSurface { header, token });
            }
        }
    }

    for path in &observation.tracked_paths {
        if let Some(reason) = forbidden_release_input(path) {
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

fn read_required(
    workspace_root: &Path,
    relative_path: &'static str,
) -> Result<String, ReleaseScopeError> {
    fs::read_to_string(workspace_root.join(relative_path)).map_err(|source| {
        ReleaseScopeError::ReadFile {
            path: relative_path,
            source,
        }
    })
}

fn has_cmake_command(text: &str, command: &str) -> bool {
    text.lines().any(|line| {
        let code = line.split_once('#').map_or(line, |(code, _)| code).trim();
        code.get(..command.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(command))
            && code[command.len()..].trim_start().starts_with('(')
    })
}

fn has_cmake_token(text: &str, token: &str) -> bool {
    text.lines().any(|line| {
        let code = line.split_once('#').map_or(line, |(code, _)| code);
        code.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .any(|word| word.eq_ignore_ascii_case(token))
    })
}

fn forbidden_release_input(path: &str) -> Option<&'static str> {
    for (prefix, reason) in FORBIDDEN_PREFIXES {
        if path == prefix.trim_end_matches('/') || path.starts_with(prefix) {
            return Some(reason);
        }
    }

    let file_name = path.rsplit('/').next().unwrap_or(path);
    if file_name == ".DS_Store" || file_name.ends_with(".rs.bk") {
        return Some("generated editor or operating-system file");
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
    if is_payload && !allowed_fixture_payload(path) {
        return Some("unapproved archive, model, or native binary payload");
    }

    None
}

fn allowed_fixture_payload(path: &str) -> bool {
    (path.starts_with("fixtures/assets/g-014/")
        && path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("zip")))
        || ALLOWED_ONNX_FIXTURES.contains(&path)
}
