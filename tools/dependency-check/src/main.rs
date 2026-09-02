//! Command-line entry point for MadoPilot repository checks.
//!
//! Exit status:
//!
//! - `0`: every requested check is compliant.
//! - `1`: at least one policy violation was found.
//! - `2`: the checker could not inspect the workspace.

use std::path::PathBuf;
use std::process::ExitCode;

use mado_pilot_dependency_check::graph::{self, Violation};
use mado_pilot_dependency_check::{metadata, release};

const USAGE: &str = "\
Check the MadoPilot workspace architecture and optional release scope.

Usage:
  mado-pilot-dependency-check [options]

Options:
  --manifest-path <PATH>  Workspace manifest to inspect (default: discovered from
                          the current directory)
  --release-scope         Validate tracked v0.4.0 release inputs and exclusions
  -q, --quiet             Print violations only
  -h, --help              Print this message

Exit status:
  0  compliant
  1  policy violations found
  2  the workspace could not be inspected
";

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}");
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let observation = match metadata::read_workspace(options.manifest_path.as_deref()) {
        Ok(observation) => observation,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };

    let mut violations = graph::validate(&observation.graph);
    violations.extend(graph::validate_metadata(
        &observation.workspace,
        &observation.members,
    ));
    violations.extend(observation.source_violations.iter().cloned());
    violations.extend(metadata::deferred_directory_violations(
        &observation.workspace_root,
    ));

    let release_violations = if options.release_scope {
        let release_observation = match release::read_workspace(&observation.workspace_root) {
            Ok(observation) => observation,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::from(2);
            }
        };
        release::validate(&release_observation)
    } else {
        Vec::new()
    };

    if violations.is_empty() && release_violations.is_empty() {
        if !options.quiet {
            report_compliant(&observation, options.release_scope);
        }
        return ExitCode::SUCCESS;
    }

    report_violations(&violations, &release_violations);
    ExitCode::from(1)
}

fn report_compliant(observation: &metadata::WorkspaceObservation, release_scope: bool) {
    let packages = observation.graph.packages();
    let edges = observation.graph.edges();
    println!(
        "architecture check passed: {} workspace packages, {} MadoPilot dependency edges",
        packages.len(),
        edges.len()
    );
    let workspace = &observation.workspace;
    println!(
        "  workspace metadata: version {}, edition {}, rust-version {}, license {}, toolchain {}",
        workspace.version.as_deref().unwrap_or("unset"),
        workspace.edition.as_deref().unwrap_or("unset"),
        workspace.rust_version.as_deref().unwrap_or("unset"),
        workspace.license.as_deref().unwrap_or("unset"),
        workspace.toolchain_channel.as_deref().unwrap_or("unset"),
    );
    for package in packages {
        println!("  {} ({})", package.name, package.directory);
    }
    for edge in edges {
        println!("  {} -> {} ({})", edge.from, edge.to, edge.kind);
    }
    if release_scope {
        println!(
            "  release scope: v{} tracked source-only inputs compliant",
            release::RELEASE_VERSION
        );
    }
}

fn report_violations(violations: &[Violation], release_violations: &[release::ReleaseViolation]) {
    for violation in violations {
        eprintln!("error: {violation}");
    }
    for violation in release_violations {
        eprintln!("error: {violation}");
    }
    let count = violations.len() + release_violations.len();
    let noun = if count == 1 {
        "violation"
    } else {
        "violations"
    };
    let check = match (violations.is_empty(), release_violations.is_empty()) {
        (false, true) => "architecture check",
        (true, false) => "release scope check",
        (false, false) => "repository check",
        (true, true) => unreachable!("a compliant result is reported before this function"),
    };
    eprintln!("{check} failed: {count} {noun}");
    eprintln!("see `docs/architecture.md` for the enforced repository contract");
}

#[derive(Debug, Default)]
struct Options {
    manifest_path: Option<PathBuf>,
    release_scope: bool,
    quiet: bool,
}

impl Options {
    /// Parses arguments, returning `Ok(None)` when usage was requested.
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self::default();
        let mut arguments = arguments.into_iter();

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "-h" | "--help" => return Ok(None),
                "-q" | "--quiet" => options.quiet = true,
                "--release-scope" => options.release_scope = true,
                "--manifest-path" => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "`--manifest-path` requires a path".to_owned())?;
                    options.manifest_path = Some(PathBuf::from(value));
                }
                other => {
                    if let Some(value) = other.strip_prefix("--manifest-path=") {
                        options.manifest_path = Some(PathBuf::from(value));
                    } else {
                        return Err(format!("unrecognized argument `{other}`"));
                    }
                }
            }
        }

        Ok(Some(options))
    }
}
