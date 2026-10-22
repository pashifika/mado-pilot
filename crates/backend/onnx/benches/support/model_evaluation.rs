//! Bounded licensed-corpus observations; rule results are evaluated externally.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mado_pilot_backend_onnx::benchmark_instrumentation::install_native_run_gate;
use mado_pilot_backend_onnx::{OnnxBackendObservations, OnnxOcrBackend};
use mado_pilot_capture::{Frame, PixelFormat};
use mado_pilot_core::{
    CancellationToken, ClipPolicy, CoordinateSpace, OperationContext, PixelExtent, PixelRect, Rect,
};
use mado_pilot_ocr::catalog::{OcrProfileDescriptor, ResourceDescriptor, ocr_profile};
use mado_pilot_ocr::{
    OcrBackend, OcrCpuOptions, OcrRecognizer, OcrRegion, OcrRequest, OcrZone, OcrZoneScanRequest,
    RecognizedRegion,
};
use mado_pilot_testkit::ManualClock;
use mado_pilot_testkit::bench_harness::{self, Plan, Sample};
use mado_pilot_testkit::vision_contract;
use opencv::core::{Mat, MatTraitConst, MatTraitConstManual};
use opencv::imgcodecs::{IMREAD_COLOR, imread};
use opencv::imgproc::{COLOR_BGR2BGRA, cvt_color_def};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod build_provenance {
    include!(concat!(env!("OUT_DIR"), "/model_evaluation_build.rs"));
}

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
type Output = Mutex<BoundedOutput>;

struct BoundedOutput {
    writer: BufWriter<File>,
    remaining: usize,
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.remaining == 0 && !bytes.is_empty() {
            return Err(std::io::Error::other(
                "evaluation report output limit exceeded",
            ));
        }
        let length = self
            .writer
            .write(&bytes[..bytes.len().min(self.remaining)])?;
        self.remaining -= length;
        Ok(length)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

const HELP: &str = "onnx-cpu --model-evaluation --profile ID --model-root DIR --runtime FILE \
  --fixtures PREPARED_JSON --binding JSON --report NEW_JSONL --process-index N --output-limit-bytes N --lifecycle-bound-ms N [--cpu-intra-threads 1..8] [--cpu-spinning true|false]\n\n\
Runs actual Rust singular and grouped OCR on every frozen licensed image.\n\
Each workload always has two warmups and twenty measured samples; CPU options do not change library defaults.\n\
The report records all outcomes, including below-threshold regions and failures.\n\
Evaluate the observations with tools/ocr-models/run.py.\n\
Metadata only (no initialization/inference): --model-evaluation --describe-profile --profile ID\n";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    fixtures: Vec<ImageSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageSpec {
    id: String,
    file: PathBuf,
    sha256: String,
    byte_len: u64,
    width: u32,
    height: u32,
    regions: Vec<Expected>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    id: String,
    order: usize,
    text_nfc: String,
    source_relative_quad: [[f64; 2]; 4],
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(text, "{byte:02x}").expect("writing a String cannot fail");
    }
    text
}

fn digest(path: &Path) -> Fallible<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0_u8; 65_536];
    loop {
        let length = file.read(&mut bytes)?;
        if length == 0 {
            return Ok(hex(&digest.finalize()));
        }
        digest.update(&bytes[..length]);
    }
}

fn resource(resource: &ResourceDescriptor, kind: &str) -> Value {
    json!({"kind": kind, "relative_path": resource.relative_path,
           "bytes": resource.byte_len, "sha256": hex(&resource.sha256)})
}

fn identity(profile: &OcrProfileDescriptor) -> Value {
    let mut resources = vec![
        resource(profile.detector, "detector"),
        resource(profile.recognizer, "recognizer"),
    ];
    if let Some(auxiliary) = profile.auxiliary {
        resources.push(resource(auxiliary, "auxiliary"));
    }
    json!({"profile_id": profile.profile_id, "model_id": profile.model_id,
           "model_version": profile.model_version, "preprocessing_id": profile.preprocessing_id,
           "decoder_id": profile.decoder_id, "normalization_id": profile.normalization_id,
           "execution_sha256": hex(&profile.execution_sha256), "resources": resources,
           "limits": {"detector_width":profile.detection.max_width, "detector_height":profile.detection.max_height,
                      "tensor_bytes":profile.limits.input_tensor_bytes}})
}

fn build_facts() -> Fallible<Value> {
    // Bind the build validator's exact source bytes without parsing a second catalog.
    Ok(
        json!({"catalog_sha256":hex(&mado_pilot_ocr::catalog::CATALOG_SOURCE_SHA256),
        "executable_sha256":digest(&std::env::current_exe()?)?,
        "build_source_revision":build_provenance::BUILD_SOURCE_REVISION,
        "build_source_tree":build_provenance::BUILD_SOURCE_TREE,
        "build_compiler":build_provenance::BUILD_COMPILER,
        "build_source_dirty":build_provenance::BUILD_SOURCE_DIRTY,
        "opencv":opencv::core::get_version_string()?}),
    )
}

fn emit(output: &Output, value: &Value) -> Fallible<()> {
    let mut output = output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn path(options: &BTreeMap<String, String>, name: &str) -> Fallible<PathBuf> {
    Ok(
        PathBuf::from(options.get(name).ok_or_else(|| format!("missing {name}"))?)
            .canonicalize()?,
    )
}

fn read_json(path: &Path) -> Fallible<Value> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("evaluation input exceeds 4 MiB".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub(super) fn run() -> Fallible<()> {
    let mut arguments = std::env::args()
        .skip(1)
        .filter(|argument| argument != "--model-evaluation" && argument != "--bench");
    let mut options = BTreeMap::new();
    let mut describe = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(());
            }
            "--describe-profile" => describe = true,
            "--profile"
            | "--model-root"
            | "--runtime"
            | "--fixtures"
            | "--binding"
            | "--report"
            | "--process-index"
            | "--output-limit-bytes"
            | "--lifecycle-bound-ms"
            | "--cpu-intra-threads"
            | "--cpu-spinning" => {
                let value = arguments.next().ok_or("option requires a value")?;
                if options.insert(argument, value).is_some() {
                    return Err("duplicate option".into());
                }
            }
            _ => return Err(format!("unknown evaluation option {argument}").into()),
        }
    }
    let profile_id = options.get("--profile").ok_or("missing --profile")?;
    let profile = ocr_profile(profile_id).ok_or("undeclared profile")?;
    let intra_threads = options
        .get("--cpu-intra-threads")
        .map_or(Ok(1), |value| value.parse::<u32>())?;
    let allow_spinning = options
        .get("--cpu-spinning")
        .map_or(Ok(true), |value| value.parse::<bool>())?;
    let cpu_options = OcrCpuOptions::new(intra_threads)?.with_spinning(allow_spinning);
    let build = build_facts()?;
    if describe {
        println!(
            "{}",
            json!({"identity":identity(profile), "build":build, "cpu_intra_threads":intra_threads, "cpu_spinning":allow_spinning})
        );
        return Ok(());
    }
    let runtime = path(&options, "--runtime")?;
    let model_root = path(&options, "--model-root")?;
    let fixture_path = path(&options, "--fixtures")?;
    let binding = read_json(&path(&options, "--binding")?)?;
    if let Some(bound_threads) = binding.get("intra_op")
        && (bound_threads.as_u64() != Some(u64::from(intra_threads)) || !allow_spinning)
    {
        return Err("CPU options differ from the bound thread/spinning cohort".into());
    }
    for name in [
        "executable_sha256",
        "catalog_sha256",
        "opencv",
        "build_source_revision",
        "build_source_tree",
        "build_compiler",
        "build_source_dirty",
    ] {
        if binding[name] != build[name] {
            return Err(format!("binding mismatch: {name}").into());
        }
    }
    for (name, actual) in [
        ("runtime_sha256", digest(&runtime)?),
        ("fixtures_sha256", digest(&fixture_path)?),
    ] {
        if binding[name].as_str() != Some(actual.as_str()) {
            return Err(format!("binding mismatch: {name}").into());
        }
    }
    let corpus: Corpus = serde_json::from_value(read_json(&fixture_path)?)?;
    if corpus.fixtures.is_empty() {
        return Err("empty corpus".into());
    }
    let process_index: u32 = options
        .get("--process-index")
        .ok_or("missing --process-index")?
        .parse()?;
    let limit: usize = options
        .get("--output-limit-bytes")
        .ok_or("missing --output-limit-bytes")?
        .parse()?;
    if limit == 0 {
        return Err("zero report output bound".into());
    }
    let lifecycle_ms: u64 = options
        .get("--lifecycle-bound-ms")
        .ok_or("missing --lifecycle-bound-ms")?
        .parse()?;
    if lifecycle_ms == 0 {
        return Err("zero lifecycle bound".into());
    }
    let lifecycle_bound = Duration::from_millis(lifecycle_ms);
    let output = Mutex::new(BoundedOutput {
        writer: BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(options.get("--report").ok_or("missing --report")?)?,
        ),
        remaining: limit,
    });
    emit(
        &output,
        &json!({"type": "meta", "index": process_index, "pid": std::process::id(),
        "binding": binding, "profile": profile_id, "identity": identity(profile),
        "build":build,
        "target": bench_harness::RELEASE_TARGET, "opencv": binding["opencv"],
        "runtime": "1.29.0-api17", "provider": "cpu"}),
    )?;
    emit(
        &output,
        &json!({"type":"collector-scope", "rust_accounting":"SDK plus raw observation construction/serialization",
        "native_peak_rss":"entire fresh process including cold initialization, workloads and lifecycle"}),
    )?;
    let operation = OperationContext::new();
    let started = Instant::now();
    let opened = OnnxOcrBackend::open_profile_with_cpu_options(
        profile_id,
        &model_root,
        &runtime,
        cpu_options,
        &operation,
    );
    let cold_open_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let backend = match opened {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            emit(
                &output,
                &json!({"type":"failure", "stage":"initialization", "status":format!("{:?}", error.status()), "fault":format!("{error:?}"), "cold_open_ms":cold_open_ms}),
            )?;
            return Err(error.into());
        }
    };
    if backend.facts().profile_id() != profile_id {
        return Err("opened profile differs from selected descriptor".into());
    }
    let vocabulary_supported = corpus
        .fixtures
        .iter()
        .flat_map(|image| &image.regions)
        .try_fold(true, |supported, region| {
            backend
                .supports_text(&region.text_nfc)
                .map(|next| supported && next)
        })?;
    emit(
        &output,
        &json!({"type":"vocabulary", "supported":vocabulary_supported}),
    )?;
    let erased: Arc<dyn OcrBackend> = backend.clone();
    let recognizer = OcrRecognizer::new(erased);
    let plan = Plan::new(2, 20);
    let mut lifecycle_frame = None;
    for image in &corpus.fixtures {
        if image.file.metadata()?.len() != image.byte_len || digest(&image.file)? != image.sha256 {
            return Err("fixture bytes differ from frozen manifest".into());
        }
        for (order, expected) in image.regions.iter().enumerate() {
            if expected.order != order
                || expected.id.is_empty()
                || expected
                    .source_relative_quad
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            {
                return Err("invalid authored oracle".into());
            }
        }
        let bgr = imread(
            image.file.to_str().ok_or("non-UTF8 fixture path")?,
            IMREAD_COLOR,
        )?;
        let mut bgra = Mat::default();
        cvt_color_def(&bgr, &mut bgra, COLOR_BGR2BGRA)?;
        if !bgra.is_continuous()
            || u32::try_from(bgra.cols())? != image.width
            || u32::try_from(bgra.rows())? != image.height
        {
            return Err("fixture dimensions differ from frozen manifest".into());
        }
        let frame = vision_contract::frame_with_pixels(
            PixelExtent::new(image.width, image.height),
            PixelFormat::Bgra8,
            bgra.data_bytes()?.to_vec(),
        );
        if lifecycle_frame.is_none() {
            lifecycle_frame = Some(frame.clone());
        }
        for grouped in [false, true] {
            let workload = format!(
                "{}/{}",
                image.id,
                if grouped { "grouped" } else { "singular" }
            );
            let before = bench_harness::live_allocated_bytes();
            let measured = bench_harness::measure(
                "onnx-model-evaluation",
                "raw licensed fixture outputs; external exact quality oracle",
                plan,
                || Fixture {
                    recognizer: &recognizer,
                    backend: &backend,
                    frame: &frame,
                    image,
                    workload: &workload,
                    grouped,
                    output: &output,
                    iteration: Mutex::new(0),
                },
                sample,
            );
            let retained = bench_harness::live_allocated_bytes().saturating_sub(before);
            emit(
                &output,
                &json!({"type":"workload", "workload":workload,
                "metrics":{"rust_peak_allocated_bytes": measured.peak_allocated_bytes(),
                           "rust_growth_bytes":measured.growth_bytes(), "rust_steady_bytes":retained}}),
            )?;
        }
    }
    let first_frame = lifecycle_frame.ok_or("empty lifecycle corpus")?;
    let full = Rect::new(
        CoordinateSpace::CapturePixels,
        0.0,
        0.0,
        f64::from(corpus.fixtures[0].width),
        f64::from(corpus.fixtures[0].height),
    )?;
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = OperationContext::new().with_cancellation(token);
    let (cancelled_status, cancellation_detail) = admitted_interruption(
        &recognizer,
        &backend,
        &first_frame,
        full,
        lifecycle_bound,
        false,
    )?;
    let (deadline_status, deadline_detail) = admitted_interruption(
        &recognizer,
        &backend,
        &first_frame,
        full,
        lifecycle_bound,
        true,
    )?;
    let initialization_cancelled = match OnnxOcrBackend::open_profile_with_cpu_options(
        profile_id,
        &model_root,
        &runtime,
        cpu_options,
        &cancelled,
    ) {
        Ok(unexpected) => {
            unexpected.close(&operation)?;
            "Ok".to_owned()
        }
        Err(error) => format!("{:?}", error.status()),
    };
    let close_started = Instant::now();
    let close = backend.close(&operation);
    let close_ms = close_started.elapsed().as_secs_f64() * 1_000.0;
    let close_idempotent = match backend.close(&operation) {
        Ok(()) => "Ok".to_owned(),
        Err(error) => format!("{:?}", error.status()),
    };
    let closed = terminal(&recognizer, &first_frame, &operation);
    let close_fault = close.err().map(|error| format!("{:?}", error.status()));
    emit(
        &output,
        &json!({"type":"terminal", "lifecycle":{"cancelled":cancelled_status,
        "deadline":deadline_status, "closed":closed, "close_idempotent":close_idempotent,
        "initialization_cancelled":initialization_cancelled}, "close_fault":close_fault,
        "lifecycle_detail":{"cancelled":cancellation_detail, "deadline":deadline_detail},
        "runtime_profile_id":backend.facts().runtime_profile_id(),
        "metrics":{"cold_open_ms":cold_open_ms, "close_ms":close_ms,
        "peak_resident_bytes":super::peak_resident_bytes(),
        "model_bytes":profile.detector.byte_len + profile.recognizer.byte_len,
        "auxiliary_bytes":profile.auxiliary.map_or(0, |resource| resource.byte_len)}}),
    )?;
    Ok(())
}

fn admitted_interruption(
    recognizer: &OcrRecognizer,
    backend: &OnnxOcrBackend,
    frame: &Frame,
    full: Rect,
    bound: Duration,
    deadline: bool,
) -> Fallible<(String, Value)> {
    let before = backend.observations()?;
    let token = CancellationToken::new();
    let clock = Arc::new(ManualClock::new());
    let context = OperationContext::new()
        .with_clock(clock.clone())
        .with_cancellation(token.clone())
        .with_timeout(bound)?;
    let recognizer = recognizer.clone();
    let frame = frame.clone();
    let gate = install_native_run_gate();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let descriptor = recognizer.descriptor();
        let zones = [OcrZone::new(full, ClipPolicy::Reject); 2];
        let request = OcrZoneScanRequest::new(
            &frame,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            &zones,
            CoordinateSpace::CapturePixels,
            &context,
        )
        .expect("two valid fixture zones");
        let status = match recognizer.scan_zones(request) {
            Ok(_) => "Ok".to_owned(),
            Err(error) => format!("{:?}", error.status()),
        };
        let _sent = sender.send(status);
    });
    let admitted = gate.wait_until_admitted(bound);
    let started = Instant::now();
    if deadline && admitted {
        clock.advance(bound);
    } else {
        token.cancel();
    }
    let terminated = admitted && gate.wait_until_termination_issued(bound);
    gate.release();
    let returned = receiver.recv_timeout(bound.saturating_sub(started.elapsed()));
    let joined = returned.is_ok() && worker.join().is_ok();
    let status = returned.unwrap_or_else(|error| format!("CollectorFailure({error})"));
    drop(gate);
    let after = backend.observations()?;
    let detail = json!({"scope":"admitted-before-native-entry", "admitted":admitted,
        "termination_issued":terminated, "worker_joined":joined,
        "return_ms":started.elapsed().as_secs_f64() * 1_000.0, "bound_ms":bound.as_millis(),
        "cleanup_completions":after.cleanup_completions().saturating_sub(before.cleanup_completions())});
    Ok((status, detail))
}

struct Fixture<'a> {
    recognizer: &'a OcrRecognizer,
    backend: &'a OnnxOcrBackend,
    frame: &'a Frame,
    image: &'a ImageSpec,
    workload: &'a str,
    grouped: bool,
    output: &'a Output,
    iteration: Mutex<usize>,
}

fn regions(regions: &[RecognizedRegion], width: u32, height: u32) -> Vec<Value> {
    regions.iter().map(|region| json!({"text":region.text(), "confidence":region.confidence().get(),
        "quad":region.geometry().points().map(|point| [point.x() / f64::from(width), point.y() / f64::from(height)])})).collect()
}

fn terminal(recognizer: &OcrRecognizer, frame: &Frame, operation: &OperationContext) -> String {
    let descriptor = recognizer.descriptor();
    match recognizer.recognize(OcrRequest::new(
        frame,
        descriptor.backend_identity(),
        descriptor.model_identity(),
        OcrRegion::FullFrame,
        CoordinateSpace::CapturePixels,
        operation,
    )) {
        Ok(_) => "Ok".to_owned(),
        Err(error) => format!("{:?}", error.status()),
    }
}

fn source_record(
    stamp: impl std::fmt::Debug,
    transform: &impl std::fmt::Debug,
    region: PixelRect,
    backend: &impl std::fmt::Debug,
    space: CoordinateSpace,
    zones: Option<&[PixelRect]>,
) -> Value {
    json!({"stamp":format!("{stamp:?}"), "transform":format!("{transform:?}"),
        "effective_region":format!("{region:?}"), "backend":format!("{backend:?}"),
        "output_space":format!("{space:?}"), "effective_zones":format!("{zones:?}")})
}

fn sample(fixture: &Fixture<'_>) -> Sample {
    let before = fixture.backend.observations();
    let descriptor = fixture.recognizer.descriptor();
    let operation = OperationContext::new();
    let extent = PixelRect::new(
        0,
        0,
        i32::try_from(fixture.image.width).expect("validated width fits i32"),
        i32::try_from(fixture.image.height).expect("validated height fits i32"),
    )
    .expect("validated dimensions");
    let full = Rect::new(
        CoordinateSpace::CapturePixels,
        0.0,
        0.0,
        f64::from(fixture.image.width),
        f64::from(fixture.image.height),
    )
    .expect("validated dimensions");
    let zones = [OcrZone::new(full, ClipPolicy::Reject); 2];
    let effective_zones = [extent; 2];
    let source = source_record(
        fixture.frame.stamp(),
        fixture.frame.transform(),
        extent,
        &descriptor,
        CoordinateSpace::CapturePixels,
        fixture.grouped.then_some(effective_zones.as_slice()),
    );
    let mut result_source = Value::Null;
    let started = Instant::now();
    // Keep JSON allocation, formatting and file output outside the SDK latency span.
    let (elapsed, status, correlation, observed, groups) = if fixture.grouped {
        let request = OcrZoneScanRequest::new(
            fixture.frame,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            &zones,
            CoordinateSpace::CapturePixels,
            &operation,
        )
        .expect("two valid zones");
        let result = fixture.recognizer.scan_zones(request);
        let elapsed = started.elapsed();
        match result {
            Ok(result) => {
                let valid = result.stamp() == fixture.frame.stamp()
                    && result.transform() == fixture.frame.transform()
                    && result.source_envelope() == extent
                    && result.effective_zones() == [extent; 2]
                    && result.output_space() == CoordinateSpace::CapturePixels
                    && result.backend() == &descriptor;
                result_source = source_record(
                    result.stamp(),
                    result.transform(),
                    result.source_envelope(),
                    result.backend(),
                    result.output_space(),
                    Some(result.effective_zones()),
                );
                let groups: Vec<Vec<usize>> = (0..2)
                    .map(|index| {
                        result
                            .group(index)
                            .expect("two groups")
                            .iter()
                            .map(|region| {
                                result
                                    .unique_candidates()
                                    .binary_search_by_key(
                                        &std::ptr::from_ref(region),
                                        std::ptr::from_ref,
                                    )
                                    .expect("group borrows unique candidate")
                            })
                            .collect()
                    })
                    .collect();
                (
                    elapsed,
                    "Ok".to_owned(),
                    valid,
                    regions(
                        result.unique_candidates(),
                        fixture.image.width,
                        fixture.image.height,
                    ),
                    Some(groups),
                )
            }
            Err(error) => (
                elapsed,
                format!("{:?}", error.status()),
                false,
                Vec::new(),
                None,
            ),
        }
    } else {
        let result = fixture.recognizer.recognize(OcrRequest::new(
            fixture.frame,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &operation,
        ));
        let elapsed = started.elapsed();
        match result {
            Ok(result) => {
                let valid = result.stamp() == fixture.frame.stamp()
                    && result.transform() == fixture.frame.transform()
                    && result.effective_region() == extent
                    && result.output_space() == CoordinateSpace::CapturePixels
                    && result.backend() == &descriptor;
                result_source = source_record(
                    result.stamp(),
                    result.transform(),
                    result.effective_region(),
                    result.backend(),
                    result.output_space(),
                    None,
                );
                (
                    elapsed,
                    "Ok".to_owned(),
                    valid,
                    regions(result.regions(), fixture.image.width, fixture.image.height),
                    None,
                )
            }
            Err(error) => (
                elapsed,
                format!("{:?}", error.status()),
                false,
                Vec::new(),
                None,
            ),
        }
    };
    let after = fixture.backend.observations();
    let mut metrics = json!({"elapsed_ms":elapsed.as_secs_f64() * 1_000.0});
    if let (Ok(before), Ok(after)) = (before, after) {
        metrics = delta(before, after, elapsed);
    }
    let mapped = metrics["mapped_bytes"].as_u64().unwrap_or(0);
    let mut iteration = fixture
        .iteration
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (phase, index) = if *iteration < 2 {
        ("warmup", *iteration)
    } else {
        ("sample", *iteration - 2)
    };
    *iteration += 1;
    let correct = status == "Ok" && correlation;
    emit(
        fixture.output,
        &json!({"type":"sample", "workload":fixture.workload,
        "phase":phase, "index":index, "status":status, "source_correlation":correlation,
        "source":source, "result_source":result_source,
        "regions":observed, "groups":groups, "metrics":metrics}),
    )
    .expect("preserve every sample or fail the process");
    Sample::new(elapsed, correct, mapped)
}

fn delta(
    before: OnnxBackendObservations,
    after: OnnxBackendObservations,
    elapsed: Duration,
) -> Value {
    let nanos = |after: u64, before: u64| {
        Duration::from_nanos(after.saturating_sub(before)).as_secs_f64() * 1_000.0
    };
    json!({"elapsed_ms":elapsed.as_secs_f64() * 1_000.0,
        "mapped_bytes":after.mapped_bytes().saturating_sub(before.mapped_bytes()),
        "detector_width":after.latest_detector_width(), "detector_height":after.latest_detector_height(),
        "detector_tensor_bytes":after.detector_tensor_bytes().saturating_sub(before.detector_tensor_bytes()),
        "detector_runs":after.detector_runs().saturating_sub(before.detector_runs()),
        "recognizer_runs":after.recognizer_runs().saturating_sub(before.recognizer_runs()),
        "detector_preprocess_ms":nanos(after.detector_preprocess_nanos(), before.detector_preprocess_nanos()),
        "detector_inference_ms":nanos(after.detector_inference_nanos(), before.detector_inference_nanos()),
        "detector_postprocess_ms":nanos(after.detector_postprocess_nanos(), before.detector_postprocess_nanos()),
        "recognizer_preprocess_ms":nanos(after.recognizer_preprocess_nanos(), before.recognizer_preprocess_nanos()),
        "recognizer_inference_ms":nanos(after.recognizer_inference_nanos(), before.recognizer_inference_nanos()),
        "recognizer_decode_ms":nanos(after.recognizer_decode_nanos(), before.recognizer_decode_nanos())})
}
