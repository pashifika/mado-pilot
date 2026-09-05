//! Direct Rust facade consumer for exact matching, CPU OCR, interruption and retained ownership.
//! Emits content-free outcomes. `qualification_module` builds the same checks behind a private
//! `mado_profile_rust_probe` seam; that experiment is not a supported facade loading path.
//! Scene arithmetic matches `deterministic-scene.h` without linking development-only testkit.

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    CancellationToken, ClipPolicy, Continuity, CoordinateSpace, DefaultOcrConfig, FindRequest,
    Frame, FrameDescriptor, FrameRequest, FrameStamp, MatchOptions, MatchResult, MonotonicInstant,
    OcrBackendDescriptor, OcrRegion, OcrRequest, OcrResult, OpenRequest, OperationContext,
    PackageSource, PixelExtent, PixelFormat, PixelRect, Rect, ReplayEngineRequest, Status,
};

/// The pixel layout the scene replay publishes; deliberately not the backend's.
const SCENE_FORMAT: PixelFormat = PixelFormat::Rgba8;

/// The accepted blank OCR fixture: a zeroed 64x64 BGRA frame.
const BLANK_EXTENT: u32 = 64;
const BLANK_FORMAT: PixelFormat = PixelFormat::Bgra8;

/// The deterministic Phase 1 scene: a pseudo-random background with a bordered,
/// graduated 12x10 patch planted at two offsets and a half-contrast copy at a
/// third. Every value is integer arithmetic on the pixel coordinate.
mod scene {
    pub const WIDTH: u32 = 96;
    pub const HEIGHT: u32 = 64;
    pub const PATCH_WIDTH: u32 = 12;
    pub const PATCH_HEIGHT: u32 = 10;
    /// Where exact copies of the patch are planted, in capture pixels.
    pub const PLANTED: [(u32, u32); 2] = [(20, 12), (60, 40)];
    /// Where the half-contrast copy the default threshold rejects is planted.
    const DEGRADED: (u32, u32) = (20, 44);
    /// Packed RGBA8 length.
    pub const BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

    /// Returns the scene as packed row-major RGBA8 with opaque alpha.
    pub fn rgba() -> Vec<u8> {
        let mut out = Vec::with_capacity(BYTES);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let [red, green, blue] = pixel(x, y);
                out.extend_from_slice(&[red, green, blue, 0xff]);
            }
        }
        out
    }

    fn pixel(x: u32, y: u32) -> [u8; 3] {
        if let Some((px, py)) = PLANTED.iter().find_map(|&origin| covers(origin, x, y)) {
            return patch(px, py);
        }
        match covers(DEGRADED, x, y) {
            Some((px, py)) => {
                let ideal = patch(px, py);
                let noise = background(x, y);
                [
                    midpoint(ideal[0], noise[0]),
                    midpoint(ideal[1], noise[1]),
                    midpoint(ideal[2], noise[2]),
                ]
            }
            None => background(x, y),
        }
    }

    /// Returns `(x, y)` relative to `origin` when a patch planted there covers it.
    fn covers(origin: (u32, u32), x: u32, y: u32) -> Option<(u32, u32)> {
        let (left, top) = origin;
        let inside = x >= left && y >= top && x < left + PATCH_WIDTH && y < top + PATCH_HEIGHT;
        inside.then(|| (x - left, y - top))
    }

    /// The patch: a white border around a two-axis gradient.
    fn patch(x: u32, y: u32) -> [u8; 3] {
        if x == 0 || y == 0 || x + 1 == PATCH_WIDTH || y + 1 == PATCH_HEIGHT {
            return [0xff, 0xff, 0xff];
        }
        [byte(x * 20), byte(y * 24), 0x30]
    }

    /// The background: reproducible noise, so nothing correlates with it by accident.
    fn background(x: u32, y: u32) -> [u8; 3] {
        let mixed = x
            .wrapping_mul(2_654_435_761)
            .wrapping_add(y.wrapping_mul(2_246_822_519));
        let mixed = mixed ^ (mixed >> 15);
        [byte(mixed), byte(mixed >> 8), byte(mixed >> 16)]
    }

    fn midpoint(first: u8, second: u8) -> u8 {
        byte((u32::from(first) + u32::from(second)) / 2)
    }

    fn byte(value: u32) -> u8 {
        u8::try_from(value & 0xff).expect("a masked value is one byte")
    }
}

/// The controlled inputs every run needs.
struct Arguments {
    package: PathBuf,
    model_root: PathBuf,
    runtime: PathBuf,
}

impl Arguments {
    /// Parses `--package`, `--model-root`, and `--runtime`, each exactly once.
    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Option<Self> {
        let mut package = None;
        let mut model_root = None;
        let mut runtime = None;
        while let Some(flag) = arguments.next() {
            let slot = match flag.to_str()? {
                "--package" => &mut package,
                "--model-root" => &mut model_root,
                "--runtime" => &mut runtime,
                _ => return None,
            };
            if slot.replace(PathBuf::from(arguments.next()?)).is_some() {
                return None;
            }
        }
        Some(Self {
            package: package?,
            model_root: model_root?,
            runtime: runtime?,
        })
    }
}

/// The first check that did not hold, printed as fixed tokens.
enum Failure {
    /// A facade call was refused; only its public status is kept.
    Refused { check: &'static str, status: Status },
    /// A call succeeded with an outcome the fixture rules out.
    Check(&'static str),
    /// The argument list did not name every controlled input exactly once.
    Usage,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Refused { check, status } => write!(
                formatter,
                "MADO_PROFILE_FAILURE={check}\nMADO_PROFILE_STATUS={}",
                status.as_str()
            ),
            Failure::Check(check) => write!(formatter, "MADO_PROFILE_FAILURE={check}"),
            Failure::Usage => formatter.write_str("MADO_PROFILE_FAILURE=usage"),
        }
    }
}

/// Names the check a facade refusal belongs to, keeping only its status.
fn step<T, E: Into<mado_pilot::Error>>(
    check: &'static str,
    result: Result<T, E>,
) -> Result<T, Failure> {
    result.map_err(|error| Failure::Refused {
        check,
        status: error.into().status(),
    })
}

fn require(condition: bool, check: &'static str) -> Result<(), Failure> {
    if condition {
        Ok(())
    } else {
        Err(Failure::Check(check))
    }
}

/// Requires a call to have been refused with exactly `expected`.
fn refused<T>(
    check: &'static str,
    result: mado_pilot::Result<T>,
    expected: Status,
) -> Result<(), Failure> {
    match result {
        Err(error) if error.status() == expected => Ok(()),
        Err(error) => Err(Failure::Refused {
            check,
            status: error.status(),
        }),
        Ok(_) => Err(Failure::Check(check)),
    }
}

/// A static replay source's first publication: epoch 0, sequence 0, geometry 0.
fn first_publication(stamp: FrameStamp) -> bool {
    stamp.epoch().value() == 0 && stamp.sequence().value() == 0 && stamp.geometry().value() == 0
}

fn same_rect(rect: PixelRect, left: i32, top: i32, right: i32, bottom: i32) -> bool {
    rect.left() == left && rect.top() == top && rect.right() == right && rect.bottom() == bottom
}

/// A context whose token was cancelled before the call, and one whose deadline
/// already passed when it was taken.
fn refusing_contexts() -> (OperationContext, OperationContext) {
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);
    let expired = OperationContext::new();
    let now = expired.now();
    (cancelled, expired.with_deadline(now))
}

/// Both planted copies, each reported once at its planted origin with the patch
/// extent, under the searched frame's identity.
fn planted_found(result: &MatchResult, stamp: FrameStamp) -> bool {
    let matches = result.matches();
    let extent = |bounds: PixelRect| {
        bounds.width() == scene::PATCH_WIDTH && bounds.height() == scene::PATCH_HEIGHT
    };
    let origin = |(x, y): (u32, u32)| {
        matches
            .iter()
            .filter(|found| {
                i32::try_from(x) == Ok(found.bounds().left())
                    && i32::try_from(y) == Ok(found.bounds().top())
            })
            .count()
            == 1
    };
    result.stamp() == stamp
        && matches.len() == scene::PLANTED.len()
        && same_rect(
            result.searched(),
            0,
            0,
            scene::WIDTH as i32,
            scene::HEIGHT as i32,
        )
        && matches
            .iter()
            .all(|found| found.template().as_str() == "panel.patch" && extent(found.bounds()))
        && scene::PLANTED.iter().all(|&planted| origin(planted))
}

fn nothing_found(result: &MatchResult, stamp: FrameStamp) -> bool {
    result.stamp() == stamp
        && result.is_empty()
        && same_rect(
            result.searched(),
            0,
            0,
            scene::WIDTH as i32,
            scene::HEIGHT as i32,
        )
}

/// An empty recognition of `region`, correlated with the recognized frame.
fn empty_recognition(result: &OcrResult, stamp: FrameStamp, region: (i32, i32, i32, i32)) -> bool {
    let (left, top, right, bottom) = region;
    result.stamp() == stamp
        && result.is_empty()
        && result.output_space() == CoordinateSpace::CapturePixels
        && same_rect(result.effective_region(), left, top, right, bottom)
        && cpu_identity(result.backend())
}

fn cpu_identity(selected: &OcrBackendDescriptor) -> bool {
    selected.id().as_str() == "onnxruntime-cpu"
        && selected.version().as_str() == "0.4.0+ort-1.29.0-api17"
        && selected.model().as_str() == "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1"
        && selected.model_identity().version().as_str()
            == "rapidocr-3.9.2+095232a4c94f7f0e6600ba5bba1177010ad696d4"
        && selected.profile().as_str() == "g-004-rapidocr-ppocrv4-det-v6-rec-small-v1"
}

/// One recognition of `region` of `frame` by the engine's selected OCR backend.
fn recognition<'a>(
    frame: &'a Frame,
    selected: &'a OcrBackendDescriptor,
    region: OcrRegion,
    operation: &'a OperationContext,
) -> OcrRequest<'a> {
    OcrRequest::new(
        frame,
        selected.backend_identity(),
        selected.model_identity(),
        region,
        CoordinateSpace::CapturePixels,
        operation,
    )
}

/// Builds a one-frame replay source publishing `pixels` under `name`.
fn replay_source(
    check: &'static str,
    name: &str,
    extent: PixelExtent,
    format: PixelFormat,
    pixels: Vec<u8>,
) -> Result<ReplaySource, Failure> {
    let descriptor = step(check, FrameDescriptor::packed(extent, format))?;
    let frame = step(
        check,
        ReplayFrame::new(
            descriptor,
            MonotonicInstant::ORIGIN,
            Continuity::Continuous,
            None,
            pixels.into_boxed_slice(),
        ),
    )?;
    let target = step(check, ReplayTarget::new(name, vec![frame]))?;
    step(check, ReplaySource::from_targets(vec![target]))
}

/// The deterministic matching workflow, with its refusal, close, and retention checks.
fn matching(package: &Path, mapping_only: bool) -> Result<(), Failure> {
    let operation = OperationContext::new();
    let scene = scene::rgba();
    let source = replay_source(
        "matching-engine",
        "panel",
        PixelExtent::new(scene::WIDTH, scene::HEIGHT),
        SCENE_FORMAT,
        scene.clone(),
    )?;
    let engine = step("matching-engine", mado_pilot::replay_engine(source))?;

    let targets = step("matching-discover", engine.discover(&operation))?;
    require(targets.len() == 1, "matching-discover")?;
    let session = step(
        "matching-open",
        engine.open(targets[0].id(), &OpenRequest::new(), &operation),
    )?;

    let frame = step(
        "matching-frame",
        session.acquire_frame(&FrameRequest::latest(), &operation),
    )?;
    let stamp = frame.stamp();
    require(first_publication(stamp), "matching-frame")?;
    let mapping = step(
        "matching-map",
        session.map_frame(&frame, SCENE_FORMAT, &operation),
    )?;
    require(
        mapping.stamp() == stamp && mapping.bytes() == scene.as_slice(),
        "matching-map",
    )?;
    if mapping_only {
        step("mapping-close", session.close(&operation))?;
        drop(frame);
        drop(session);
        drop(engine);
        require(
            mapping.stamp() == stamp && mapping.bytes() == scene.as_slice(),
            "mapping-retained",
        )?;
        println!("MADO_PROFILE_MAPPING=retained");
        return Ok(());
    }
    drop(mapping);

    let package = step(
        "package",
        engine.load_package(&PackageSource::directory(package), &operation),
    )?;
    require(package.template_count() == 2, "package")?;
    let present = step(
        "template-present",
        engine.prepare_from_package(&package, "panel.patch", &operation),
    )?;
    let absent = step(
        "template-absent",
        engine.prepare_from_package(&package, "panel.absent", &operation),
    )?;

    let search_present = FindRequest::exact(
        &frame,
        &present,
        MatchOptions::from_defaults(present.defaults()),
    );
    let found = step(
        "find-present",
        session.find_template(&search_present, &operation),
    )?;
    require(planted_found(found.result(), stamp), "find-present")?;
    let missing = step(
        "find-absent",
        session.find_template(
            &FindRequest::exact(
                &frame,
                &absent,
                MatchOptions::from_defaults(absent.defaults()),
            ),
            &operation,
        ),
    )?;
    require(nothing_found(missing.result(), stamp), "find-absent")?;
    println!("MADO_PROFILE_MATCHING=passed");

    let (cancelled, expired) = refusing_contexts();
    refused(
        "cancellation",
        session.find_template(&search_present, &cancelled),
        Status::Cancelled,
    )?;
    println!("MADO_PROFILE_CANCELLATION=refused");
    refused(
        "deadline",
        session.find_template(&search_present, &expired),
        Status::DeadlineExceeded,
    )?;
    println!("MADO_PROFILE_DEADLINE=refused");
    let both = cancelled.with_deadline(expired.now());
    refused(
        "cancellation-precedence",
        session.find_template(&search_present, &both),
        Status::Cancelled,
    )?;

    step("close", session.close(&operation))?;
    step("close", session.close(&operation))?;
    require(session.is_closed(), "close")?;
    refused(
        "close",
        session.acquire_frame(&FrameRequest::latest(), &operation),
        Status::Closed,
    )?;
    println!("MADO_PROFILE_CLOSE=idempotent");

    // What the caller owns survives every producer.
    drop(present);
    drop(absent);
    drop(package);
    drop(frame);
    drop(session);
    drop(engine);
    require(
        planted_found(found.result(), stamp) && nothing_found(missing.result(), stamp),
        "retained",
    )?;
    println!("MADO_PROFILE_RETAINED=readable");
    Ok(())
}

/// The accepted CPU blank-frame OCR workflow, with the same refusal, close, and
/// retention checks.
fn ocr(model_root: &Path, runtime: &Path) -> Result<(), Failure> {
    let config = DefaultOcrConfig::new(
        model_root
            .canonicalize()
            .map_err(|_| Failure::Check("ocr-prerequisite"))?,
        runtime
            .canonicalize()
            .map_err(|_| Failure::Check("ocr-prerequisite"))?,
    );
    let operation = OperationContext::new();
    let extent = PixelExtent::new(BLANK_EXTENT, BLANK_EXTENT);
    let blank = vec![0; BLANK_EXTENT as usize * BLANK_EXTENT as usize * 4];
    let source = replay_source(
        "ocr-engine",
        "default-ocr-blank",
        extent,
        BLANK_FORMAT,
        blank,
    )?;
    let engine = step(
        "ocr-engine",
        mado_pilot::replay_engine_with_default_ocr(
            ReplayEngineRequest::new(source),
            &config,
            &operation,
        ),
    )?;
    let selected = engine.ocr_backend().ok_or(Failure::Check("ocr-engine"))?;
    require(cpu_identity(&selected), "ocr-identity")?;
    let provider = engine
        .ocr_provider()
        .ok_or(Failure::Check("ocr-provider"))?;
    require(
        provider.active_provider() == mado_pilot::OcrExecutionProvider::Cpu
            && provider.requested_policy() == mado_pilot::OcrExecutionProviderPolicy::Cpu
            && !provider.initialization_fell_back()
            && provider.runtime_profile().as_str() == "onnxruntime-1.29.0-api17-cpu",
        "ocr-provider",
    )?;

    let targets = step("ocr-discover", engine.discover(&operation))?;
    require(targets.len() == 1, "ocr-discover")?;
    let session = step(
        "ocr-open",
        engine.open(targets[0].id(), &OpenRequest::new(), &operation),
    )?;
    let frame = step(
        "ocr-frame",
        session.acquire_frame(&FrameRequest::latest(), &operation),
    )?;
    let stamp = frame.stamp();
    require(first_publication(stamp), "ocr-frame")?;

    let whole = i32::try_from(BLANK_EXTENT).map_err(|_| Failure::Check("ocr-full"))?;
    let full = step(
        "ocr-full",
        session.recognize(recognition(
            &frame,
            &selected,
            OcrRegion::FullFrame,
            &operation,
        )),
    )?;
    require(
        empty_recognition(&full, stamp, (0, 0, whole, whole)),
        "ocr-full",
    )?;
    let bounded_region = OcrRegion::Region {
        rect: step(
            "ocr-region",
            Rect::new(CoordinateSpace::CapturePixels, 8.0, 8.0, 40.0, 40.0),
        )?,
        policy: ClipPolicy::Reject,
    };
    let bounded = step(
        "ocr-region",
        session.recognize(recognition(&frame, &selected, bounded_region, &operation)),
    )?;
    require(
        empty_recognition(&bounded, stamp, (8, 8, 40, 40)),
        "ocr-region",
    )?;

    let (cancelled, expired) = refusing_contexts();
    refused(
        "ocr-cancellation",
        session.recognize(recognition(
            &frame,
            &selected,
            OcrRegion::FullFrame,
            &cancelled,
        )),
        Status::Cancelled,
    )?;
    refused(
        "ocr-deadline",
        session.recognize(recognition(
            &frame,
            &selected,
            OcrRegion::FullFrame,
            &expired,
        )),
        Status::DeadlineExceeded,
    )?;
    let both = cancelled.with_deadline(expired.now());
    refused(
        "ocr-cancellation-precedence",
        session.recognize(recognition(&frame, &selected, OcrRegion::FullFrame, &both)),
        Status::Cancelled,
    )?;

    step("ocr-close", session.close(&operation))?;
    step("ocr-close", session.close(&operation))?;
    require(session.is_closed(), "ocr-close")?;
    refused(
        "ocr-close",
        session.acquire_frame(&FrameRequest::latest(), &operation),
        Status::Closed,
    )?;

    drop(frame);
    drop(session);
    drop(selected);
    drop(engine);
    require(
        empty_recognition(&full, stamp, (0, 0, whole, whole))
            && empty_recognition(&bounded, stamp, (8, 8, 40, 40)),
        "ocr-retained",
    )?;
    println!("MADO_PROFILE_OCR=passed");
    Ok(())
}

/// Runs every check against the controlled inputs, printing one observation
/// line per passed group. The terminal line belongs to the caller.
fn run(arguments: &Arguments) -> Result<(), Failure> {
    println!("MADO_PROFILE_CONSUMER=rust-facade");
    matching(&arguments.package, false)?;
    matching(&arguments.package, true)?;
    ocr(&arguments.model_root, &arguments.runtime)
}

/// Parses the process arguments, runs the checks, and prints the terminal line:
/// 0 when every check held, 1 otherwise.
fn execute() -> i32 {
    let outcome = Arguments::parse(env::args_os().skip(1))
        .ok_or(Failure::Usage)
        .and_then(|arguments| run(&arguments));
    match outcome {
        Ok(()) => {
            println!("MADO_PROFILE_RESULT=passed");
            0
        }
        Err(failure) => {
            if matches!(failure, Failure::Usage) {
                eprintln!("usage: --package <dir> --model-root <dir> --runtime <file>");
            }
            println!("{failure}");
            println!("MADO_PROFILE_RESULT=failed");
            1
        }
    }
}

/// [`execute`], with a contained panic reported as 2 instead of an unwind. The
/// panic path writes without `println!`, which could panic again on a closed
/// stream and unwind through the probe's `extern "C"` boundary.
fn guarded() -> i32 {
    std::panic::catch_unwind(execute).unwrap_or_else(|_| {
        let _ = writeln!(
            io::stdout(),
            "MADO_PROFILE_FAILURE=panic\nMADO_PROFILE_RESULT=failed"
        );
        2
    })
}

#[cfg(not(qualification_module))]
fn main() {
    std::process::exit(guarded());
}

/// Private probe entry for the deferred-load experiment: the same checks, read
/// from the host process's own argument list. 0 passed, 1 failed, 2 panicked.
#[cfg(qualification_module)]
#[unsafe(no_mangle)]
pub extern "C" fn mado_profile_rust_probe() -> i32 {
    guarded()
}
