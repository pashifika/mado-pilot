//! Finite selected-layout, persistent-hit and genuine active-eviction observations.

use std::time::Duration;

use mado_pilot::{
    ClipPolicy, Frame, FrameDescriptor, MatchResult, OperationContext, PixelFormat,
    PreparedTemplate, RegionSelection,
};
use mado_pilot_backend_opencv::benchmark_instrumentation as backend;
use mado_pilot_testkit::MatchRequest;
use serde_json::{Value, json};

use super::fixtures::{self, Workload};

const WAIT: Duration = Duration::from_secs(5);
const FULL_BGR: u64 = 1920 * 1080 * 3;
const ROI_BGR: u64 = 320 * 180 * 3;

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    source: PixelFormat,
    padding: usize,
    roi: bool,
    forced: bool,
}

const CASES: [Case; 10] = [
    Case {
        id: "rgba-full",
        source: PixelFormat::Rgba8,
        padding: 0,
        roi: false,
        forced: false,
    },
    Case {
        id: "bgra-full",
        source: PixelFormat::Bgra8,
        padding: 0,
        roi: false,
        forced: false,
    },
    Case {
        id: "rgba-padded-full",
        source: PixelFormat::Rgba8,
        padding: 16,
        roi: false,
        forced: false,
    },
    Case {
        id: "bgra-padded-full",
        source: PixelFormat::Bgra8,
        padding: 16,
        roi: false,
        forced: false,
    },
    Case {
        id: "rgba-roi",
        source: PixelFormat::Rgba8,
        padding: 0,
        roi: true,
        forced: false,
    },
    Case {
        id: "bgra-roi",
        source: PixelFormat::Bgra8,
        padding: 0,
        roi: true,
        forced: false,
    },
    Case {
        id: "rgba-padded-roi",
        source: PixelFormat::Rgba8,
        padding: 16,
        roi: true,
        forced: false,
    },
    Case {
        id: "bgra-padded-roi",
        source: PixelFormat::Bgra8,
        padding: 16,
        roi: true,
        forced: false,
    },
    Case {
        id: "rgba-forced-bgra-full",
        source: PixelFormat::Rgba8,
        padding: 0,
        roi: false,
        forced: true,
    },
    Case {
        id: "rgba-forced-bgra-roi",
        source: PixelFormat::Rgba8,
        padding: 0,
        roi: true,
        forced: true,
    },
];

pub(super) fn manifest() -> Value {
    let cases: Vec<_> = CASES.iter().map(|case| {
        let pixels = if case.roi { 320 * 180 } else { 1920 * 1080 };
        json!({
            "id": case.id, "source_format": case.source.as_str(),
            "selected_format": if case.forced { "bgra8" } else { case.source.as_str() },
            "row_padding_bytes": case.padding, "nonzero_roi": case.roi,
            "forced_bgra_public_mapping": case.forced,
            "expected_matches": 1, "expected_bounds": [80, 64, 92, 74],
            "expected_mapping_copies": if case.roi || case.forced { 1 } else { 0 },
            "expected_mapping_copied_bytes": if case.roi || case.forced { pixels * 4 } else { 0 },
            "expected_bgr_conversions": 1, "expected_bgr_converted_bytes": pixels * 3,
        })
    }).collect();
    json!({
        "cases": cases,
        "transition_cases": [
            {"id": "same-source-roi", "selected_format": "rgba8", "region": [64, 48, 384, 228], "expected_bounds": [80, 64, 92, 74],
                "mapping_copies": 1, "mapping_copied_bytes": 230400, "bgr_conversions": 1, "bgr_converted_bytes": 172800},
            {"id": "same-source-alternate-roi", "selected_format": "rgba8", "region": [72, 56, 376, 220], "expected_bounds": [152, 64, 164, 74],
                "mapping_copies": 1, "mapping_copied_bytes": 199424, "bgr_conversions": 1, "bgr_converted_bytes": 149568},
            {"id": "distinct-bgra-full", "selected_format": "bgra8", "region": [0, 0, 1920, 1080], "expected_bounds": [80, 64, 92, 74],
                "mapping_copies": 0, "mapping_copied_bytes": 0, "bgr_conversions": 1, "bgr_converted_bytes": 6220800},
        ],
        "lifetime_order": ["persistent-hit", "active-evicted"],
        "persistent_checkpoint_order": ["mapping-live", "cold-result", "persistent-hit", "mapping-released", "frame-released", "result-released", "final-backend-released"],
        "active_checkpoint_order": ["prepared-reader-paused", "active-image-evicted", "final-reader-completed", "frames-released", "results-released", "final-backend-released"],
        "expected_per_iteration": {
            "backend_find_calls": 18, "bgr_conversions": 17,
            "bgr_converted_bytes": 57_346_368, "mapping_copies": 10,
            "mapping_copied_bytes": 10_337_024,
        },
        "operation_timeout_seconds": 5, "prepared_reader_pause_timeout_seconds": 5,
        "resource_scope": "logical mapping owners and owned BGR payload; not total native allocation or RSS release",
        "active_owner_order": "final production reader completes before final backend release",
    })
}

fn operation() -> OperationContext {
    OperationContext::new()
        .with_timeout(WAIT)
        .expect("finite format operation")
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn frame(row: Workload, case: Case, variant: u32) -> Result<Frame, String> {
    let pixels = fixtures::scene(row.extent(), variant, case.source);
    if case.padding == 0 {
        return super::replay_frame(row, pixels, case.source);
    }
    let stride = row.width as usize * 4 + case.padding;
    let descriptor = FrameDescriptor::new(row.extent(), case.source, stride)
        .map_err(|error| error.to_string())?;
    let mut padded = vec![0xA5; descriptor.byte_len()];
    for (source, destination) in pixels
        .chunks_exact(row.width as usize * 4)
        .zip(padded.chunks_exact_mut(stride))
    {
        destination[..source.len()].copy_from_slice(source);
    }
    super::replay_frame_with_descriptor(descriptor, padded)
}

fn request<'a>(
    frame: &'a Frame,
    template: &'a PreparedTemplate,
    row: Workload,
) -> Result<MatchRequest<'a>, String> {
    Ok(MatchRequest::new(
        frame,
        RegionSelection::pixels(super::region(row), ClipPolicy::Reject)
            .map_err(|error| error.to_string())?,
        template,
        super::options(template),
    ))
}

fn work(before: backend::WorkSnapshot) -> Value {
    let after = backend::work_snapshot();
    json!({
        "mapping_copies": after.mapping_copies - before.mapping_copies,
        "mapping_copied_bytes": after.mapping_copied_bytes - before.mapping_copied_bytes,
        "bgr_conversions": after.bgr_conversions - before.bgr_conversions,
        "bgr_converted_bytes": after.bgr_converted_bytes - before.bgr_converted_bytes,
    })
}

fn checkpoint(name: &str, mapping_live: bool, source_alive: bool, result_live: bool) -> Value {
    let state = backend::work_snapshot();
    json!({"checkpoint": name, "mapping_owner_live": mapping_live,
        "source_frame_alive": source_alive, "result_owner_live": result_live,
        "cache_held_bgr_bytes": state.bgr_retained_bytes,
        "active_evicted_bgr_bytes": state.bgr_evicted_bytes,
        "active_backend_finds": backend::snapshot().active_finds})
}

fn checked_result(
    result: &MatchResult,
    frame: &Frame,
    template: &PreparedTemplate,
    row: Workload,
    variant: u32,
) -> Result<(), String> {
    require(
        super::check(result, frame, template, super::region(row), variant, 0, row),
        "format/lifetime public result oracle",
    )
}

fn persistent(row: Workload, template: &PreparedTemplate) -> Result<Value, String> {
    let matcher = super::matcher()?;
    let frame = frame(row, CASES[4], 0)?;
    let before = backend::work_snapshot();
    let mapped = matcher
        .map_match(&request(&frame, template, row)?, &operation())
        .map_err(|error| error.to_string())?;
    let source = mapped
        .pixels()
        .ok_or("persistent mapping missing")?
        .source_instance()
        .clone();
    let mut checkpoints = vec![checkpoint("mapping-live", true, source.is_alive(), false)];
    let cold = matcher
        .find_mapped(&mapped, template, &operation())
        .map_err(|error| error.to_string())?;
    checked_result(&cold, &frame, template, row, 0)?;
    checkpoints.push(checkpoint("cold-result", true, source.is_alive(), true));
    let result = matcher
        .find_mapped(&mapped, template, &operation())
        .map_err(|error| error.to_string())?;
    checked_result(&result, &frame, template, row, 0)?;
    require(
        work(before)["bgr_conversions"] == 1 && work(before)["mapping_copies"] == 1,
        "persistent hit repeats physical preparation",
    )?;
    checkpoints.push(checkpoint("persistent-hit", true, source.is_alive(), true));
    drop(cold);
    drop(mapped);
    let released_mapping = backend::work_snapshot();
    let probe = mado_pilot::FrameView::new(frame.clone(), super::region(row))
        .map_err(|error| error.to_string())?
        .map(PixelFormat::Rgba8, &operation())
        .map_err(|error| error.to_string())?;
    require(
        backend::work_snapshot().mapping_copies == released_mapping.mapping_copies + 1,
        "cache/result retains the released ROI mapping payload",
    )?;
    drop(probe);
    checkpoints.push(checkpoint(
        "mapping-released",
        false,
        source.is_alive(),
        true,
    ));
    drop(frame);
    require(!source.is_alive(), "BGR cache/result retains source frame")?;
    require(
        backend::work_snapshot().bgr_retained_bytes == ROI_BGR,
        "persistent payload missing before final backend release",
    )?;
    checkpoints.push(checkpoint("frame-released", false, source.is_alive(), true));
    drop(result);
    checkpoints.push(checkpoint(
        "result-released",
        false,
        source.is_alive(),
        false,
    ));
    drop(matcher);
    checkpoints.push(checkpoint(
        "final-backend-released",
        false,
        source.is_alive(),
        false,
    ));
    require(
        backend::work_snapshot().bgr_retained_bytes == 0
            && backend::work_snapshot().bgr_evicted_bytes == 0,
        "persistent final BGR owner survives",
    )?;
    Ok(json!({"id": "persistent-hit", "checkpoints": checkpoints, "work": work(before)}))
}

fn active(mut row: Workload, template: &PreparedTemplate) -> Result<Value, String> {
    row.roi = false;
    let matcher = super::matcher()?;
    let frames = [
        frame(row, CASES[0], 0)?,
        frame(row, CASES[0], 1)?,
        frame(row, CASES[0], 2)?,
    ];
    let sources = frames
        .iter()
        .map(|frame| {
            frame
                .map(PixelFormat::Rgba8, &operation())
                .map(|pixels| pixels.source_instance().clone())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let before = backend::work_snapshot();
    let pause = backend::install_prepared_reader_pause()
        .ok_or("prepared-reader observation already owned")?;
    let mut checkpoints = Vec::new();
    let result = std::thread::scope(|scope| -> Result<MatchResult, String> {
        let worker = scope.spawn(|| {
            super::find(
                &matcher,
                &frames[0],
                template,
                super::region(row),
                &operation(),
            )
        });
        let pressure = (|| -> Result<(), String> {
            require(
                pause.wait_ready(),
                "prepared-reader gate not reached within deadline",
            )?;
            require(
                pause
                    .admitted_payload()
                    .is_some_and(|bytes| bytes as u64 == FULL_BGR),
                "paused reader payload mismatch",
            )?;
            require(
                backend::snapshot().active_finds == 1,
                "prepared gate has no active production search",
            )?;
            checkpoints.push(checkpoint("prepared-reader-paused", true, true, false));
            for (index, frame) in frames[1..].iter().enumerate() {
                let result =
                    super::find(&matcher, frame, template, super::region(row), &operation())
                        .map_err(|error| error.to_string())?;
                checked_result(
                    &result,
                    frame,
                    template,
                    row,
                    u32::try_from(index).expect("two pressure frames") + 1,
                )?;
            }
            let state = backend::work_snapshot();
            require(
                state.bgr_retained_bytes == 2 * FULL_BGR
                    && state.bgr_evicted_bytes == FULL_BGR
                    && backend::snapshot().active_finds == 1,
                "pressure did not evict the genuinely active production reader",
            )?;
            checkpoints.push(checkpoint("active-image-evicted", true, true, false));
            Ok(())
        })();
        // Resume and join on every outcome, including a failed pressure oracle.
        pause.resume();
        let result = worker
            .join()
            .map_err(|_| "paused production reader panicked".to_owned())?
            .map_err(|error| error.to_string());
        pressure?;
        result
    })?;
    drop(pause);
    checked_result(&result, &frames[0], template, row, 0)?;
    require(
        backend::work_snapshot().bgr_evicted_bytes == 0,
        "evicted pixels survive final production reader",
    )?;
    checkpoints.push(checkpoint("final-reader-completed", false, true, true));
    drop(frames);
    require(
        sources.iter().all(|source| !source.is_alive()),
        "active lifecycle retains a source frame",
    )?;
    checkpoints.push(checkpoint("frames-released", false, false, true));
    drop(result);
    checkpoints.push(checkpoint("results-released", false, false, false));
    drop(matcher);
    checkpoints.push(checkpoint("final-backend-released", false, false, false));
    require(
        backend::work_snapshot().bgr_retained_bytes == 0
            && backend::work_snapshot().bgr_evicted_bytes == 0,
        "active lifecycle final BGR owner survives",
    )?;
    Ok(json!({"id": "active-evicted", "checkpoints": checkpoints, "work": work(before)}))
}

fn transitions(
    row: Workload,
    first: &PreparedTemplate,
    second: &PreparedTemplate,
    declaration: &Value,
) -> Result<Value, String> {
    let matcher = super::matcher()?;
    let frames = [frame(row, CASES[0], 0)?, frame(row, CASES[1], 0)?];
    let mut observations = Vec::with_capacity(3);
    for (index, expected) in declaration["transition_cases"]
        .as_array()
        .expect("three transitions")
        .iter()
        .enumerate()
    {
        let frame = &frames[usize::from(index == 2)];
        let (template, template_index) = if index == 1 { (second, 1) } else { (first, 0) };
        let bounds = expected["region"].as_array().expect("fixed region");
        let searched = mado_pilot::PixelRect::new(
            i32::try_from(bounds[0].as_i64().expect("left")).expect("bounded left"),
            i32::try_from(bounds[1].as_i64().expect("top")).expect("bounded top"),
            i32::try_from(bounds[2].as_i64().expect("right")).expect("bounded right"),
            i32::try_from(bounds[3].as_i64().expect("bottom")).expect("bounded bottom"),
        )
        .expect("declared transition region");
        let request = MatchRequest::new(
            frame,
            RegionSelection::pixels(searched, ClipPolicy::Reject)
                .map_err(|error| error.to_string())?,
            template,
            super::options(template),
        );
        let before = backend::work_snapshot();
        let mapped = matcher
            .map_match(&request, &operation())
            .map_err(|error| error.to_string())?;
        let selected = mapped
            .pixels()
            .ok_or("transition mapping missing")?
            .descriptor()
            .format();
        let result = matcher
            .find_mapped(&mapped, template, &operation())
            .map_err(|error| error.to_string())?;
        require(
            super::check(&result, frame, template, searched, 0, template_index, row),
            "ROI/format transition public result oracle",
        )?;
        require(
            expected["selected_format"] == selected.as_str(),
            "ROI/format transition selected layout",
        )?;
        let observed = work(before);
        for key in [
            "mapping_copies",
            "mapping_copied_bytes",
            "bgr_conversions",
            "bgr_converted_bytes",
        ] {
            require(
                observed[key] == expected[key],
                &format!("ROI/format transition: unexpected {key}"),
            )?;
        }
        observations.push(
            json!({"id": expected["id"], "selected_format": selected.as_str(),
            "region": expected["region"], "work": observed, "correct": true}),
        );
    }
    Ok(json!(observations))
}

pub(super) fn run(mut row: Workload) -> Result<Value, String> {
    let compiler = super::matcher()?;
    let sources = fixtures::templates();
    let template = compiler
        .prepare(&sources[0], &operation())
        .map_err(|error| error.to_string())?;
    let alternate = compiler
        .prepare(&sources[1], &operation())
        .map_err(|error| error.to_string())?;
    drop(compiler);
    let declaration = manifest();
    let before = backend::work_snapshot();
    let activity_before = backend::snapshot();
    let mut cases = Vec::with_capacity(CASES.len());
    for (case, expected) in CASES
        .into_iter()
        .zip(declaration["cases"].as_array().expect("fixed case array"))
    {
        row.roi = case.roi;
        let matcher = if case.forced {
            backend::forced_bgra_matcher().map_err(|error| error.to_string())?
        } else {
            super::matcher()?
        };
        let frame = frame(row, case, 0)?;
        let start = backend::work_snapshot();
        let mapped = matcher
            .map_match(&request(&frame, &template, row)?, &operation())
            .map_err(|error| error.to_string())?;
        let pixels = mapped.pixels().ok_or("format mapping missing")?;
        let selected = pixels.descriptor().format();
        require(
            selected
                == if case.forced {
                    PixelFormat::Bgra8
                } else {
                    case.source
                },
            "actual selected layout mismatch",
        )?;
        let shared = !case.roi && !case.forced;
        let expected_stride = if case.roi {
            320 * 4
        } else if case.forced {
            1920 * 4
        } else {
            frame.descriptor().stride()
        };
        require(
            pixels.is_shared() == shared && pixels.descriptor().stride() == expected_stride,
            "actual shared/padded/packed mapping layout mismatch",
        )?;
        let result = matcher
            .find_mapped(&mapped, &template, &operation())
            .map_err(|error| error.to_string())?;
        checked_result(&result, &frame, &template, row, 0)?;
        let observed = work(start);
        for key in [
            "mapping_copies",
            "mapping_copied_bytes",
            "bgr_conversions",
            "bgr_converted_bytes",
        ] {
            require(
                observed[key] == expected[format!("expected_{key}")],
                &format!("{}: unexpected {key}", case.id),
            )?;
        }
        cases.push(json!({"id": case.id, "source_format": case.source.as_str(), "selected_format": selected.as_str(),
            "source_stride": frame.descriptor().stride(), "mapped_stride": pixels.descriptor().stride(),
            "mapping_shared": pixels.is_shared(), "work": observed, "correct": true}));
    }
    let transitions = transitions(row, &template, &alternate, &declaration)?;
    row.roi = true;
    let persistent = persistent(row, &template)?;
    let active = active(row, &template)?;
    let observed = work(before);
    let activity = backend::snapshot()
        .checked_delta(activity_before)
        .ok_or("unfinished format backend work")?;
    let declared = &declaration["expected_per_iteration"];
    require(
        activity.find_calls == 18 && activity.find_completions == 18 && activity.find_failures == 0,
        "format backend work count",
    )?;
    for key in [
        "mapping_copies",
        "mapping_copied_bytes",
        "bgr_conversions",
        "bgr_converted_bytes",
    ] {
        require(
            observed[key] == declared[key],
            &format!("format iteration: unexpected {key}"),
        )?;
    }
    Ok(
        json!({"correct": true, "cases": cases, "transitions": transitions, "lifetimes": [persistent, active], "work": observed,
        "backend_find_calls": activity.find_calls, "backend_find_completions": activity.find_completions}),
    )
}
