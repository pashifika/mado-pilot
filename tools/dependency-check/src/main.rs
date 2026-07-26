//! Command-line entry point for the MadoPilot architecture checker.
//!
//! Exit status:
//!
//! - `0`: the workspace inventory and dependency directions are compliant.
//! - `1`: at least one architecture violation was found.
//! - `2`: the checker could not inspect the workspace.

use std::path::PathBuf;
use std::process::ExitCode;

use mado_pilot_dependency_check::graph::{self, Violation};
use mado_pilot_dependency_check::metadata;

const USAGE: &str = "\
Check the MadoPilot workspace package inventory and dependency directions.

Usage:
  mado-pilot-dependency-check [options]

Options:
  --manifest-path <PATH>  Workspace manifest to inspect (default: discovered from
                          the current directory)
  -q, --quiet             Print violations only
  -h, --help              Print this message

Exit status:
  0  compliant
  1  architecture violations found
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
    violations.extend(graph::validate_metadata(&observation.members));
    violations.extend(observation.source_violations.iter().cloned());
    violations.extend(metadata::deferred_directory_violations(
        &observation.workspace_root,
    ));

    if violations.is_empty() {
        if !options.quiet {
            report_compliant(&observation);
        }
        return ExitCode::SUCCESS;
    }

    report_violations(&violations);
    ExitCode::from(1)
}

fn report_compliant(observation: &metadata::WorkspaceObservation) {
    let packages = observation.graph.packages();
    let edges = observation.graph.edges();
    println!(
        "architecture check passed: {} workspace packages, {} MadoPilot dependency edges",
        packages.len(),
        edges.len()
    );
    if let Some(reference) = observation
        .members
        .iter()
        .find(|member| member.name == graph::FACADE)
    {
        println!(
            "  shared metadata: version {}, edition {}, rust-version {}, license {}",
            reference.version,
            reference.edition,
            reference.rust_version.as_deref().unwrap_or("unset"),
            reference.license.as_deref().unwrap_or("unset"),
        );
    }
    for package in packages {
        println!("  {} ({})", package.name, package.directory);
    }
    for edge in edges {
        println!("  {} -> {} ({})", edge.from, edge.to, edge.kind);
    }
}

fn report_violations(violations: &[Violation]) {
    for violation in violations {
        eprintln!("error: {violation}");
    }
    let count = violations.len();
    let noun = if count == 1 {
        "violation"
    } else {
        "violations"
    };
    eprintln!("architecture check failed: {count} {noun}");
    eprintln!("see `docs/architecture.md` for the Phase 0 inventory and dependency allowlist");
}

#[derive(Debug, Default)]
struct Options {
    manifest_path: Option<PathBuf>,
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
