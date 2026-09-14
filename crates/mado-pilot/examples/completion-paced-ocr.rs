//! Caller-owned exact-frame OCR with a full post-interpretation cooldown.
//!
//! Model-free, no-capture smoke:
//! `cargo run --locked -p mado-pilot --example completion-paced-ocr -- --controlled-smoke`
//!
//! Native mode requires explicit target/model/runtime arguments and separate
//! capture authority; see `--help` and `docs/capture-pacing.md`. No input or
//! permission request is made, and target selection has no implicit fallback.
//!
//! One absolute operation governs the loop. Owners are released before bounded
//! 2ms polling; OS wakeup latency is additional. Live updates during cooldown
//! remain eligible, but terminal sources are not drained. Already-admitted OCR
//! may finish on its held frame after capture loss. Native pacing and watcher
//! admission rate are distinct from this completion cooldown.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use mado_pilot::{
    CapturePacingRequest, DefaultOcrConfig, Engine, Error, FrameRequest, FrameStamp,
    NativeEngineRequest, OcrBackendDescriptor, OcrRegion, OcrResult, OpenRequest, OperationContext,
    Result, Session, Status, TargetDescription, TargetId, TargetKind,
};

#[path = "support/completion_cooldown.rs"]
mod completion_cooldown;
#[path = "support/completion_paced_ocr.rs"]
mod controlled;

use completion_cooldown::WAIT_SLICE;
use completion_cooldown::{checkpoint, cooldown, recognize_exact};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const USAGE: &str = "usage: completion-paced-ocr --controlled-smoke | --native <window|display> <exact-target-name> <model-root> <runtime-file> <source-default|required:MS|preferred:MS> <cooldown-ms> <operation-ms> <exact-stop-text>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Continue,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Completed {
    observations: u64,
    last: FrameStamp,
}

// Local caller policy, not a watcher or a public scheduling abstraction.
#[derive(Clone, Copy)]
struct Consumption {
    cooldown: Duration,
    region: OcrRegion,
}

fn consume(
    session: &Session,
    backend: &OcrBackendDescriptor,
    policy: Consumption,
    operation: &OperationContext,
    mut interpret: impl FnMut(&OcrResult, &OperationContext) -> Result<Decision>,
    mut sleep: impl FnMut(Duration),
) -> Result<Completed> {
    checkpoint(operation)?;
    if policy.cooldown.is_zero() {
        return Err(invalid("cooldown must be positive"));
    }
    let mut request = FrameRequest::latest();
    let mut observations = 0_u64;
    loop {
        checkpoint(operation)?;
        let (stamp, decision) = {
            let frame = session.acquire_frame(&request, operation)?;
            let result = recognize_exact(session, backend, &frame, policy.region, operation)?;
            checkpoint(operation)?;
            let decision = interpret(&result, operation)?;
            checkpoint(operation)?;
            (frame.stamp(), decision)
        }; // Result, frame, and recognition's mapping/view owners are gone.
        observations = observations
            .checked_add(1)
            .ok_or_else(|| Error::new(Status::LimitExceeded, "observation count exhausted"))?;
        if decision == Decision::Stop {
            checkpoint(operation)?;
            return Ok(Completed {
                observations,
                last: stamp,
            });
        }
        cooldown(policy.cooldown, operation, &mut sleep)?;
        // No capture-time fence: a publication during OCR or cooldown is eligible.
        request = FrameRequest::newer_than(stamp);
    }
}

fn interpret_until(
    result: &OcrResult,
    exact_text: &str,
    operation: &OperationContext,
) -> Result<Decision> {
    checkpoint(operation)?;
    let mut decision = Decision::Continue;
    // The OCR contract bounds region count and each text. Equality allocates
    // nothing; checking between regions also bounds this caller's own work.
    for region in result.regions() {
        checkpoint(operation)?;
        if region.text() == exact_text {
            decision = Decision::Stop;
            break;
        }
    }
    checkpoint(operation)?;
    Ok(decision)
}

fn close_session(session: &Session, operation: &OperationContext) -> Result<()> {
    // Cleanup must still run when the loop's token/deadline is already terminal.
    let cleanup = OperationContext::new()
        .with_clock(operation.clock())
        .with_timeout(CLOSE_TIMEOUT)?;
    session.close(&cleanup)
}

#[derive(Debug)]
enum RunFailure {
    Work(Error),
    Cleanup(Error),
    WorkAndCleanup(Error, Error),
}

impl fmt::Display for RunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Work(error) => write!(formatter, "OCR flow failed: {}", error.status()),
            Self::Cleanup(error) => {
                write!(formatter, "session cleanup incomplete: {}", error.status())
            }
            Self::WorkAndCleanup(work, cleanup) => write!(
                formatter,
                "OCR flow failed: {}; session cleanup also incomplete: {}",
                work.status(),
                cleanup.status(),
            ),
        }
    }
}

impl std::error::Error for RunFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Work(error) | Self::Cleanup(error) | Self::WorkAndCleanup(error, _) => {
                Some(error)
            }
        }
    }
}

fn with_cleanup<T>(work: Result<T>, cleanup: Result<()>) -> std::result::Result<T, RunFailure> {
    match (work, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(work), Ok(())) => Err(RunFailure::Work(work)),
        (Ok(_), Err(cleanup)) => Err(RunFailure::Cleanup(cleanup)),
        (Err(work), Err(cleanup)) => Err(RunFailure::WorkAndCleanup(work, cleanup)),
    }
}

struct NativeArguments {
    kind: TargetKind,
    name: String,
    model_root: PathBuf,
    runtime_file: PathBuf,
    pacing: CapturePacingRequest,
    cooldown: Duration,
    timeout: Duration,
    stop_text: String,
}

impl NativeArguments {
    fn parse(arguments: &mut impl Iterator<Item = OsString>) -> Result<Self> {
        let kind = match text_argument(arguments)?.as_str() {
            "window" => TargetKind::Window,
            "display" => TargetKind::Display,
            _ => return Err(invalid("target kind must be window or display")),
        };
        let name = text_argument(arguments)?;
        if name.is_empty() {
            return Err(invalid("an exact nonempty target name is required"));
        }
        let model_root = PathBuf::from(arguments.next().ok_or_else(|| invalid(USAGE))?);
        let runtime_file = PathBuf::from(arguments.next().ok_or_else(|| invalid(USAGE))?);
        let pacing_text = text_argument(arguments)?;
        let pacing = match pacing_text.as_str() {
            "source-default" => CapturePacingRequest::source_default(),
            value => {
                let (policy, milliseconds) = value.split_once(':').ok_or_else(|| {
                    invalid("native pacing must be source-default, required:MS, or preferred:MS")
                })?;
                let interval = positive_milliseconds(milliseconds)?;
                match policy {
                    "required" => CapturePacingRequest::required(interval)?,
                    "preferred" => CapturePacingRequest::preferred(interval)?,
                    _ => {
                        return Err(invalid(
                            "native pacing policy must be required or preferred",
                        ));
                    }
                }
            }
        };
        let cooldown = positive_milliseconds(&text_argument(arguments)?)?;
        let timeout = positive_milliseconds(&text_argument(arguments)?)?;
        let stop_text = text_argument(arguments)?;
        if stop_text.is_empty() || stop_text.len() > mado_pilot::MAX_TEXT_BYTES {
            return Err(invalid(
                "stop text must fit one nonempty normalized OCR region",
            ));
        }
        if arguments.next().is_some() {
            return Err(invalid(USAGE));
        }
        Ok(Self {
            kind,
            name,
            model_root,
            runtime_file,
            pacing,
            cooldown,
            timeout,
            stop_text,
        })
    }
}

fn text_argument(arguments: &mut impl Iterator<Item = OsString>) -> Result<String> {
    arguments
        .next()
        .ok_or_else(|| invalid(USAGE))?
        .into_string()
        .map_err(|_| invalid("text and numeric arguments must be valid UTF-8"))
}

fn positive_milliseconds(value: &str) -> Result<Duration> {
    let milliseconds = value
        .parse::<u64>()
        .map_err(|_| invalid("milliseconds must be a positive integer"))?;
    if milliseconds == 0 {
        return Err(invalid("milliseconds must be positive"));
    }
    Ok(Duration::from_millis(milliseconds))
}

fn invalid(detail: &'static str) -> Error {
    Error::new(Status::InvalidArgument, detail)
}

fn select_target(targets: &[TargetDescription], kind: TargetKind, name: &str) -> Result<TargetId> {
    let mut matches = targets
        .iter()
        .filter(|target| target.capability().kind() == Some(kind) && target.name() == name);
    let target = matches
        .next()
        .ok_or_else(|| invalid("no target matches the exact authorized kind and name"))?;
    if matches.next().is_some() {
        return Err(invalid(
            "target selection is ambiguous; use a unique authorized target name",
        ));
    }
    Ok(target.id())
}

fn run_native(arguments: NativeArguments) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // This single authority includes model setup, discovery, open, and every iteration.
    let operation = OperationContext::new().with_timeout(arguments.timeout)?;
    let model_root = arguments.model_root.canonicalize()?;
    let runtime_file = arguments.runtime_file.canonicalize()?;
    if !model_root.is_dir() || !runtime_file.is_file() {
        return Err(invalid("controlled model root must be a directory and runtime a file").into());
    }
    checkpoint(&operation)?;
    let config = DefaultOcrConfig::new(model_root, runtime_file);
    let engine = native_engine(
        NativeEngineRequest::new().with_capture_pacing(arguments.pacing),
        &config,
        &operation,
    )?;
    let backend = engine.ocr_backend().ok_or_else(|| {
        Error::new(
            Status::Internal,
            "default OCR constructor omitted its backend",
        )
    })?;
    let targets = engine.discover(&operation)?;
    let target = select_target(&targets, arguments.kind, &arguments.name)?;
    // Capture-only open inherits the selected engine default; no input is established.
    let session = engine.open(target, &OpenRequest::new(), &operation)?;
    let pacing = session.description().capture_pacing();
    let work = consume(
        &session,
        &backend,
        Consumption {
            cooldown: arguments.cooldown,
            region: OcrRegion::FullFrame,
        },
        &operation,
        |result, operation| interpret_until(result, &arguments.stop_text, operation),
        std::thread::sleep,
    );
    let completed = with_cleanup(work, close_session(&session, &operation))?;
    drop(session);
    drop(engine);
    // No target name, recognized text, pixels, model paths, or backend payloads.
    println!(
        "completion-paced-ocr: observations={} last={} native={:?} configured={:?}",
        completed.observations,
        completed.last,
        pacing.outcome(),
        pacing.configured_interval(),
    );
    Ok(())
}

#[cfg(windows)]
fn native_engine(
    request: NativeEngineRequest,
    config: &DefaultOcrConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    mado_pilot::windows_engine_with_default_ocr(request, config, operation)
}

#[cfg(target_os = "macos")]
fn native_engine(
    request: NativeEngineRequest,
    config: &DefaultOcrConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    mado_pilot::macos_engine_with_default_ocr(request, config, operation)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn native_engine(
    _request: NativeEngineRequest,
    _config: &DefaultOcrConfig,
    _operation: &OperationContext,
) -> Result<Engine> {
    Err(Error::new(
        Status::Unsupported,
        "native capture requires Windows or macOS",
    ))
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next().as_deref() {
        Some(mode) if mode == "--controlled-smoke" && arguments.next().is_none() => {
            controlled::smoke()
        }
        Some(mode) if mode == "--native" => run_native(NativeArguments::parse(&mut arguments)?),
        Some(mode) if mode == "--help" && arguments.next().is_none() => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(invalid(USAGE).into()),
    }
}
