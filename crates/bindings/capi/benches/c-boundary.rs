//! What a caller pays for crossing the C ABI, measured against not crossing it.
//!
//! Task 9.2 asks for "any material C ABI startup overhead". Whether it is
//! material is the question, and the only honest way to answer it is to run the
//! same work twice in one process — once through the negotiated function table
//! and once through the Rust facade — and report both. A C number on its own
//! says how fast this machine is; the pair says what the boundary costs.
//!
//! Three things are measured, each paired except the first, which has no Rust
//! counterpart because it is the boundary itself:
//!
//! | Workload | What it is |
//! |---|---|
//! | `negotiate_table` | `madopilot_get_api`, the whole of what a C caller pays before it can do anything |
//! | `engine_create_*` | Building an engine over the deterministic scene |
//! | `match_warm_*` | One search with an already-prepared template, and reading every match back |
//!
//! The Rust halves duplicate what `mado-pilot`'s own benchmark measures. That is
//! deliberate and is what makes the comparison valid: same process, same build,
//! same run, same host, so the difference between a pair is the boundary and
//! not the conditions.
//!
//! The C halves reach the library the way a C caller does — through the table
//! `madopilot_get_api` returns, with `#[repr(C)]` structures the header
//! declares — rather than through this crate's Rust items. What they do not
//! reproduce is dynamic loading: this benchmark links the library, so a `dlopen`
//! or `LoadLibrary` a real C host performs once at startup is outside every
//! measurement here and is not claimed to be measured.
//!
//! ```text
//! cargo test  --locked --workspace --all-targets        # oracles, three samples
//! cargo bench --locked --package mado-pilot-capi --bench c-boundary -- \
//!     --hardware "..." --os-version "..."               # full run, TOML report
//! ```

use std::ffi::c_char;
use std::path::PathBuf;
use std::ptr;
use std::time::Instant;

use mado_pilot::replay::{ReplayFrame, ReplaySource, ReplayTarget};
use mado_pilot::{
    ContentDigest, Continuity, Engine, FindRequest, Frame, FrameDescriptor, FrameRequest,
    MatchOptions, MonotonicInstant, OpenRequest, OperationContext, PackageSource, PixelFormat,
    PreparedTemplate, Session,
};
use mado_pilot_testkit::bench_harness::{Accounting, Benchmark, Plan, Profile, Sample, measure};
use mado_pilot_testkit::{bench_harness, match_fixtures};

use madopilot::layout::struct_size;
use madopilot::*;

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

/// Where the planted copies sit, and therefore what every search must find.
const PLANTED: [(i32, i32); 2] = [(20, 12), (60, 40)];

/// How far a score may sit from an exact correlation and still be one.
const TOLERANCE: f64 = 1e-5;

/// What both matching workloads are checked against.
const PLANTED_ORACLE: &str =
    "the two planted copies are found at their planted offsets, each scoring 1.0 within 1e-5";

/// The layout the matcher maps a searched region into.
///
/// `mapped_bytes_per_result` is the searched area times this format's size, and
/// the C half cannot derive the second factor: the C ABI reports the backend's
/// identity and version but not the layout it requires, and the ABI is frozen.
/// Naming the format here rather than writing its byte width into the
/// arithmetic is what makes the assumption visible — and
/// [`match_warm_rust`], which can ask the engine, checks it, so a backend that
/// required another layout fails this benchmark instead of having its bytes
/// reported under the old one.
const BACKEND_FORMAT: PixelFormat = PixelFormat::Bgra8;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let plan = Plan::from(&arguments);
    let (hardware, os_version) = Profile::host(&arguments);

    let workloads = [
        measure(
            "negotiate_table",
            "the table reports this header's ABI major and is at least its size",
            plan,
            || (),
            negotiate_table,
        ),
        measure(
            "engine_create_c",
            "the engine discovers the source's one target",
            plan,
            CFlow::new,
            engine_create_c,
        ),
        measure(
            "engine_create_rust",
            "the engine discovers the source's one target",
            plan,
            RustFlow::new,
            engine_create_rust,
        ),
        measure(
            "match_warm_c",
            PLANTED_ORACLE,
            plan,
            CFlow::new,
            match_warm_c,
        ),
        measure(
            "match_warm_rust",
            PLANTED_ORACLE,
            plan,
            RustFlow::new,
            match_warm_rust,
        ),
    ];

    if arguments.iter().any(|argument| argument == "--bench") {
        bench_harness::report(
            &Benchmark {
                id: "phase-1-c-boundary",
                workload: "the C ABI's cost, against the same work through the Rust facade",
                phase: "1",
            },
            &Profile {
                fixture: "fixtures/assets/phase1-slice, \
                          mado-pilot-testkit match_fixtures for the scene"
                    .to_owned(),
                fixture_sha256: fixture_digest().to_string(),
                hardware,
                os_version,
                build_profile: format!(
                    "cargo bench, default features, debug_assertions={}",
                    cfg!(debug_assertions)
                ),
                correctness_oracle: "every retained sample is checked; \
                                     each measurement states its own oracle",
                queue_policy: "none; every Phase 1 operation is synchronous \
                               and no work is queued",
                notes: None,
            },
            plan,
            &workloads,
        );
    } else {
        bench_harness::summarize("c-boundary", plan, &workloads);
    }

    // After the report, so a run that fails a gate still emits the numbers that
    // explain the failure.
    bench_harness::enforce_hard_budgets(&workloads);
}

// --- Negotiation -------------------------------------------------------------

/// What a C caller pays before it holds anything at all.
fn negotiate_table(&(): &()) -> Sample {
    let mut api: *const madopilot_api_t = ptr::null();

    let started = Instant::now();
    // SAFETY: `api` is a live, writable, correctly aligned local.
    let status = unsafe {
        madopilot_get_api(
            MADOPILOT_ABI_MAJOR,
            MADOPILOT_ABI_MINOR,
            size_of::<madopilot_api_t>(),
            &raw mut api,
        )
    };
    let elapsed = started.elapsed();

    // SAFETY: a successful negotiation returns the library's static table.
    let correct = status == MADOPILOT_STATUS_OK
        && unsafe { api.as_ref() }.is_some_and(|table| {
            table.abi_major == MADOPILOT_ABI_MAJOR
                && table.struct_size as usize >= size_of::<madopilot_api_t>()
        });

    Sample::unmapped(elapsed, correct)
}

// --- The C side ---------------------------------------------------------------

/// The deterministic scene, described the way a C caller describes one.
///
/// One value because the source points at the frame and the frame points at the
/// pixels: separating them would leave a structure whose pointees had been
/// dropped.
struct Scene {
    pixels: Vec<u8>,
    frame: madopilot_replay_frame_t,
    source: madopilot_source_t,
}

impl Scene {
    fn new() -> Box<Self> {
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let mut scene = Box::new(Self {
            pixels,
            frame: madopilot_replay_frame_t {
                struct_size: struct_size::<madopilot_replay_frame_t>(),
                flags: 0,
                width: match_fixtures::SCENE.width(),
                height: match_fixtures::SCENE.height(),
                format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                continuity: MADOPILOT_CONTINUITY_CONTINUOUS,
                pixels: madopilot_bytes_t::empty(),
                captured_at_nanos: 0,
                stride: 0,
            },
            source: madopilot_source_t {
                struct_size: struct_size::<madopilot_source_t>(),
                kind: MADOPILOT_SOURCE_REPLAY_MEMORY,
                directory: madopilot_str_t::empty(),
                frames: ptr::null(),
                frame_count: 1,
                frame_stride: size_of::<madopilot_replay_frame_t>(),
                target_name: madopilot_str_t::empty(),
            },
        });

        scene.frame.pixels = madopilot_bytes_t {
            data: scene.pixels.as_ptr(),
            len: scene.pixels.len(),
        };
        scene.source.frames = &raw const scene.frame;

        scene
    }
}

/// Everything the C workloads need that is not what they measure.
struct CFlow {
    api: &'static madopilot_api_t,
    scene: Box<Scene>,
    engine: *mut madopilot_engine_t,
    session: *mut madopilot_session_t,
    frame: *mut madopilot_frame_t,
    package: *mut madopilot_package_t,
    template: *mut madopilot_template_t,
}

impl CFlow {
    fn new() -> Self {
        let mut api: *const madopilot_api_t = ptr::null();
        // SAFETY: `api` is a live, writable, correctly aligned local.
        let status = unsafe {
            madopilot_get_api(
                MADOPILOT_ABI_MAJOR,
                MADOPILOT_ABI_MINOR,
                size_of::<madopilot_api_t>(),
                &raw mut api,
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "negotiation");
        // SAFETY: negotiation succeeded, so `api` names the library's static
        // table, which lives as long as the library does.
        let api = unsafe { api.as_ref() }.expect("a negotiated table is never null");

        let scene = Scene::new();
        // Borrowed by the package source below and not needed afterwards: the
        // load completes before this function returns.
        let root = package_root();
        let operation = operation();
        let engine = create_engine(api, &scene, &operation);

        let mut targets = ptr::null_mut();
        let mut session = ptr::null_mut();
        let mut frame = ptr::null_mut();
        let mut package = ptr::null_mut();
        let open = madopilot_open_request_t {
            struct_size: struct_size::<madopilot_open_request_t>(),
            flags: 0,
            required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
        };
        let source = madopilot_package_source_t {
            struct_size: struct_size::<madopilot_package_source_t>(),
            kind: MADOPILOT_PACKAGE_SOURCE_DIRECTORY,
            path: str_view(&root),
            archive: madopilot_bytes_t::empty(),
        };

        // SAFETY: every handle is owned here and every pointer is a live local;
        // `root` and `scene` outlive the calls.
        unsafe {
            assert_eq!(
                (api.engine_discover)(
                    engine,
                    &raw const operation,
                    &raw mut targets,
                    ptr::null_mut()
                ),
                MADOPILOT_STATUS_OK
            );
            assert_eq!(
                (api.session_open)(
                    engine,
                    targets,
                    0,
                    &raw const open,
                    &raw const operation,
                    &raw mut session,
                    ptr::null_mut(),
                ),
                MADOPILOT_STATUS_OK
            );
            (api.target_list_release)(targets);
            assert_eq!(
                (api.session_acquire_frame)(
                    session,
                    &raw const operation,
                    &raw mut frame,
                    ptr::null_mut()
                ),
                MADOPILOT_STATUS_OK
            );
            assert_eq!(
                (api.package_load)(
                    engine,
                    &raw const source,
                    &raw const operation,
                    &raw mut package,
                    ptr::null_mut(),
                ),
                MADOPILOT_STATUS_OK
            );
        }

        let mut template = ptr::null_mut();
        // SAFETY: as above.
        let status = unsafe {
            (api.template_prepare_from_package)(
                engine,
                package,
                str_view("panel.patch"),
                &raw const operation,
                &raw mut template,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK, "template_prepare_from_package");

        Self {
            api,
            scene,
            engine,
            session,
            frame,
            package,
            template,
        }
    }
}

impl Drop for CFlow {
    fn drop(&mut self) {
        // SAFETY: each handle was produced by this table and is owned here.
        unsafe {
            (self.api.template_release)(self.template);
            (self.api.package_release)(self.package);
            (self.api.frame_release)(self.frame);
            (self.api.session_release)(self.session);
            (self.api.engine_release)(self.engine);
        }
    }
}

fn engine_create_c(flow: &CFlow) -> Sample {
    let operation = operation();

    let started = Instant::now();
    let engine = create_engine(flow.api, &flow.scene, &operation);
    let elapsed = started.elapsed();

    let mut targets = ptr::null_mut();
    let mut count = 0;
    // SAFETY: the engine was just created here and every pointer is a live local.
    let correct = unsafe {
        let discovered = (flow.api.engine_discover)(
            engine,
            &raw const operation,
            &raw mut targets,
            ptr::null_mut(),
        ) == MADOPILOT_STATUS_OK
            && (flow.api.target_list_count)(targets, &raw mut count) == MADOPILOT_STATUS_OK;
        (flow.api.target_list_release)(targets);
        (flow.api.engine_release)(engine);
        discovered
    } && count == 1;

    Sample::unmapped(elapsed, correct)
}

fn match_warm_c(flow: &CFlow) -> Sample {
    let operation = operation();
    let request = madopilot_find_request_t {
        struct_size: struct_size::<madopilot_find_request_t>(),
        flags: 0,
        frame: flow.frame,
        tmpl: flow.template,
        options: ptr::null(),
        region: madopilot_pixel_rect_t::empty(),
        clip_policy: MADOPILOT_CLIP_POLICY_REJECT,
    };
    let mut result = ptr::null_mut();
    let mut info = madopilot_result_info_t {
        struct_size: struct_size::<madopilot_result_info_t>(),
        ..zeroed_result_info()
    };
    let mut found = Vec::with_capacity(2);

    // The measured window is what a C caller does to get an answer it can act
    // on: search, learn how many matches there are, and read each one out
    // through the accessor. Stopping at `session_find` would measure half of
    // what crossing the boundary costs, because the results are still inside
    // an opaque handle at that point.
    let started = Instant::now();
    // SAFETY: every handle the request names is retained by `flow`, and every
    // pointer is a live local.
    let status = unsafe {
        (flow.api.session_find)(
            flow.session,
            &raw const request,
            &raw const operation,
            &raw mut result,
            ptr::null_mut(),
        )
    };
    if status == MADOPILOT_STATUS_OK {
        // SAFETY: the search succeeded, so `result` is an owned handle.
        unsafe {
            (flow.api.result_describe)(result, &raw mut info);
            for index in 0..info.match_count {
                let mut one = madopilot_match_t {
                    struct_size: struct_size::<madopilot_match_t>(),
                    ..zeroed_match()
                };
                let at = usize::try_from(index).expect("a match count fits an index");
                if (flow.api.result_match)(result, at, &raw mut one) == MADOPILOT_STATUS_OK {
                    found.push((one.bounds.left, one.bounds.top, one.score));
                }
            }
        }
    }
    let elapsed = started.elapsed();

    // SAFETY: released exactly once, after the window that measured it.
    unsafe { (flow.api.result_release)(result) };

    Sample::new(
        elapsed,
        status == MADOPILOT_STATUS_OK && planted(&found),
        searched_bytes(&info),
    )
}

// --- The Rust side ------------------------------------------------------------

/// The same fixtures, reached through the facade.
struct RustFlow {
    engine: Engine,
    operation: OperationContext,
    session: Session,
    frame: Frame,
    template: PreparedTemplate,
    /// The scene's pixels, generated once.
    ///
    /// The C half hands `engine_create` a pointer to a buffer it already holds
    /// and the library copies it. Generating the scene inside the Rust half's
    /// measured window would put work in one side of the pair that the other
    /// side does not do, so the pixels are produced here and each iteration
    /// pays only for the copy, which is what the C side pays for too.
    pixels: Vec<u8>,
}

impl RustFlow {
    fn new() -> Self {
        let pixels = match_fixtures::scene_pixels(PixelFormat::Rgba8);
        let engine = mado_pilot::replay_engine(source_from(&pixels))
            .expect("an OpenCV 4 development installation");
        let operation = OperationContext::new();
        let targets = engine.discover(&operation).expect("discovered");
        let session = engine
            .open(targets[0].id(), &OpenRequest::new(), &operation)
            .expect("opened");
        let frame = session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .expect("a published frame");
        let package = engine
            .load_package(&PackageSource::directory(package_root()), &operation)
            .expect("the tracked example package loads");
        let template = engine
            .prepare_from_package(&package, "panel.patch", &operation)
            .expect("prepared");

        Self {
            engine,
            operation,
            session,
            frame,
            template,
            pixels,
        }
    }
}

fn engine_create_rust(flow: &RustFlow) -> Sample {
    let operation = OperationContext::new();

    let started = Instant::now();
    let engine = mado_pilot::replay_engine(source_from(&flow.pixels)).expect("built");
    let elapsed = started.elapsed();

    let correct = engine
        .discover(&operation)
        .is_ok_and(|targets| targets.len() == 1);

    Sample::unmapped(elapsed, correct)
}

fn match_warm_rust(flow: &RustFlow) -> Sample {
    let options = MatchOptions::from_defaults(flow.template.defaults());
    let mut found = Vec::with_capacity(2);

    // The same window as the C half: search, then read every match out.
    let started = Instant::now();
    let outcome = flow
        .session
        .find_template(
            &FindRequest::exact(&flow.frame, &flow.template, options),
            &flow.operation,
        )
        .expect("searched");
    for one in outcome.result().matches() {
        found.push((one.bounds().left(), one.bounds().top(), one.score()));
    }
    let elapsed = started.elapsed();

    let searched = outcome.result().searched();
    // The half of the pair that can ask. Both halves report a byte count built
    // from the same format, and the C one has to be told which; this is where
    // being told the wrong one is caught.
    let format = flow.engine.backend().format();
    assert_eq!(
        format, BACKEND_FORMAT,
        "the C half computes mapped bytes from BACKEND_FORMAT, which the C ABI \
         gives it no way to read back"
    );
    let mapped = u64::from(searched.width())
        * u64::from(searched.height())
        * u64::from(format.bytes_per_pixel());

    Sample::new(elapsed, planted(&found), mapped)
}

// --- Shared -------------------------------------------------------------------

/// Reports whether a search found exactly the planted copies.
///
/// Compared as a set. Two byte-identical copies correlate at one to within the
/// tolerance, so which of them a result puts first rests on a difference smaller
/// than the tolerance, and an oracle that asserted an order would be asserting
/// the host's rounding rather than the workload's correctness.
fn planted(found: &[(i32, i32, f64)]) -> bool {
    let mut origins: Vec<(i32, i32)> = found.iter().map(|&(left, top, _)| (left, top)).collect();
    origins.sort_unstable_by_key(|&(left, top)| (top, left));

    let mut expected = PLANTED.to_vec();
    expected.sort_unstable_by_key(|&(left, top)| (top, left));

    origins == expected
        && found
            .iter()
            .all(|&(_, _, score)| (score - 1.0).abs() <= TOLERANCE)
}

/// Returns the frame bytes one search mapped, from what the result reports.
///
/// Derived rather than observed, from the region the result says was searched
/// and the format the matcher maps into. The matcher maps that region exactly
/// once per search, so this is the rule it follows and not a reading of what it
/// did: a backend that mapped twice would break the rule, and this arithmetic
/// would go on reporting one mapping. Detecting that needs an observer inside
/// the library, which is what `mado-pilot`'s own benchmark has and a caller on
/// the far side of the C boundary cannot.
fn searched_bytes(info: &madopilot_result_info_t) -> u64 {
    let width = u64::from(
        info.searched
            .right
            .saturating_sub(info.searched.left)
            .unsigned_abs(),
    );
    let height = u64::from(
        info.searched
            .bottom
            .saturating_sub(info.searched.top)
            .unsigned_abs(),
    );

    width * height * u64::from(BACKEND_FORMAT.bytes_per_pixel())
}

fn create_engine(
    api: &'static madopilot_api_t,
    scene: &Scene,
    operation: &madopilot_operation_t,
) -> *mut madopilot_engine_t {
    let mut engine = ptr::null_mut();
    // SAFETY: `scene` outlives the call and every pointer is a live local.
    let status = unsafe {
        (api.engine_create)(
            &raw const scene.source,
            &raw const *operation,
            &raw mut engine,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, MADOPILOT_STATUS_OK, "engine_create");

    engine
}

/// An operation with no deadline and no cancellation.
fn operation() -> madopilot_operation_t {
    madopilot_operation_t {
        struct_size: struct_size::<madopilot_operation_t>(),
        flags: 0,
        deadline_nanos: 0,
        cancellation: ptr::null(),
        activity_tag: 0,
    }
}

fn str_view(value: &str) -> madopilot_str_t {
    madopilot_str_t {
        data: value.as_ptr().cast::<c_char>(),
        len: value.len(),
    }
}

fn zeroed_result_info() -> madopilot_result_info_t {
    madopilot_result_info_t {
        struct_size: 0,
        flags: 0,
        match_count: 0,
        backend_id: madopilot_str_t::empty(),
        backend_version: madopilot_str_t::empty(),
        searched: madopilot_pixel_rect_t::empty(),
    }
}

fn zeroed_match() -> madopilot_match_t {
    madopilot_match_t {
        struct_size: 0,
        flags: 0,
        score: 0.0,
        template_id: madopilot_str_t::empty(),
        bounds: madopilot_pixel_rect_t::empty(),
    }
}

/// Builds a replay source over a copy of `pixels`.
///
/// The copy is deliberate and is inside every caller's measured window: it is
/// the same copy `engine_create` makes of the buffer a C caller points it at.
fn source_from(pixels: &[u8]) -> ReplaySource {
    let descriptor = FrameDescriptor::packed(match_fixtures::SCENE, PixelFormat::Rgba8)
        .expect("a valid descriptor");
    let frame = ReplayFrame::new(
        descriptor,
        MonotonicInstant::ORIGIN,
        Continuity::Continuous,
        None,
        pixels.to_vec().into_boxed_slice(),
    )
    .expect("a valid replay frame");

    ReplaySource::from_targets(vec![
        ReplayTarget::new("panel", vec![frame]).expect("a valid target"),
    ])
    .expect("a valid source")
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
}

fn package_root() -> String {
    fixtures()
        .join("assets/phase1-slice")
        .to_string_lossy()
        .into_owned()
}

/// The digest of the one fixture set this benchmark reads.
fn fixture_digest() -> ContentDigest {
    let sums = fixtures().join("assets/phase1-slice/SHA256SUMS");
    let bytes = std::fs::read(&sums).unwrap_or_else(|error| {
        panic!(
            "{} is a tracked fixture checksum file: {error}",
            sums.display()
        )
    });

    ContentDigest::of(&bytes)
}
