//! Explicit host policy over the public facade; help and smoke create no engine.

mod config;
mod decisions;
mod flow;
mod smoke;

use std::path::PathBuf;
use std::process::ExitCode;

use mado_pilot::{Error, InputFault};

const HELP: &str = "mado-pilot-input-workflow
  --help
  --smoke
  --config PATH --allow-capture [--allow-input]

No arguments refuse before engine construction. Help and smoke do not load
models, discover targets, probe permissions, capture, or submit input.
Operational use requires an explicit JSON config and capture consent. Actions
also require separate input consent; action.kind=observe opens capture only.
Only an exact unique window title is accepted. No activation, route fallback,
input retry, downloads, or permission prompts are provided.
JSON is limited to 64 KiB and rejects unknown fields. See the usage guide for
schema, native prerequisites, finite bounds, and source-only readiness.
Native pacing is configuration, watcher rate is backend admission, and neither
is a completion cooldown. A receipt is not application consumption; a newer
postcondition is not proof of causation. Geometry checks are not atomic visual
observe-and-act. Logical close does not prove physical backend quiescence.";

enum Command {
    Help,
    Smoke,
    Run { path: PathBuf, allow_input: bool },
}

#[derive(Debug)]
pub(crate) enum Failure {
    Policy(&'static str),
    Library {
        stage: &'static str,
        error: Error,
    },
    Input {
        stage: &'static str,
        fault: InputFault,
    },
}

impl Failure {
    fn library(stage: &'static str, error: Error) -> Self {
        Self::Library { stage, error }
    }

    fn report(&self) {
        match self {
            Self::Policy(stage) => eprintln!("failure: stage={stage}"),
            Self::Library { stage, error } => {
                // Native/model details may contain private paths. Keep the typed category.
                eprintln!("failure: stage={stage} status={}", error.status());
            }
            Self::Input { stage, fault } => {
                eprintln!("failure: stage={stage} fault={fault}");
            }
        }
    }
}

type Result<T, E = Failure> = std::result::Result<T, E>;

fn command(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(Failure::Policy(
            "explicit_config_and_capture_consent_required",
        ));
    };
    if first == "--help" || first == "--smoke" {
        if args.next().is_some() {
            return Err(Failure::Policy("help_and_smoke_take_no_other_arguments"));
        }
        return Ok(if first == "--help" {
            Command::Help
        } else {
            Command::Smoke
        });
    }
    let mut path = None;
    let mut capture = false;
    let mut input = false;
    let mut next = Some(first);
    while let Some(arg) = next {
        if arg == "--config" && path.is_none() {
            path = Some(PathBuf::from(
                args.next().ok_or(Failure::Policy("missing_config_path"))?,
            ));
        } else if arg == "--allow-capture" && !capture {
            capture = true;
        } else if arg == "--allow-input" && !input {
            input = true;
        } else {
            return Err(Failure::Policy("unknown_or_duplicate_argument"));
        }
        next = args.next();
    }
    if !capture {
        return Err(Failure::Policy("capture_consent_required"));
    }
    Ok(Command::Run {
        path: path.ok_or(Failure::Policy("config_required"))?,
        allow_input: input,
    })
}

fn run() -> Result<bool> {
    match command(std::env::args_os().skip(1))? {
        Command::Help => {
            println!("{HELP}");
            Ok(true)
        }
        Command::Smoke => {
            smoke::run()?;
            println!(
                "smoke: passed authority/source/coordinates/recipes/partial-effect decisions; native_operations=0"
            );
            Ok(true)
        }
        Command::Run { path, allow_input } => {
            let config = config::load(&path)?;
            decisions::authorize(&config.action, allow_input)?;
            flow::run(&config, allow_input)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            error.report();
            ExitCode::FAILURE
        }
    }
}
