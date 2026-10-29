//! Rust OCR text presence on fixed replay pixels with independent wait and result lifetimes.

#[cfg(windows)]
#[path = "support/ocr_dependency_images.rs"]
mod ocr_dependency_images;

#[cfg(feature = "ocr-text-watch-qualification")]
#[path = "support/ocr_watch_measurements.rs"]
mod ocr_watch_measurements;

#[cfg(feature = "ocr-text-watch-qualification")]
use ocr_watch_measurements::{Measurements, Stage};

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mado_pilot::replay::ReplaySource;
use mado_pilot::{
    CancellationToken, ChangeDetectionPolicy, ClipPolicy, CoordinateSpace, OcrExecutionProvider,
    OcrProfile, OcrProfileConfig, OcrTextAnalysisRate, OcrTextStability, OcrTextTerminalOutcome,
    OcrTextWatchRequest, OcrTextWatchResult, OpenRequest, OperationContext, PixelFormat, PixelRect,
    Rect, ReplayEngineRequest, Status,
};

const EXPECTED_TEXT: [&str; 8] = [
    "魔導士",
    "Lv.42",
    "HP1234/5678",
    "MP98%",
    "クエスト",
    "[A-7]",
    "次へ>",
    "READY!",
];

// Epoch, sequence, geometry of the first replay source.
const FIRST_SOURCE_IDENTITY: (u64, u64, u64) = (0, 0, 0);
const TRANSITION_SEQUENCE: u64 = 1;

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            #[cfg(windows)]
            if let Err(error) = ocr_dependency_images::record_if_requested() {
                eprintln!(
                    "ocr-text-watch: dependency observation failed: {error}; no dependency proof is implied"
                );
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            let status = error
                .downcast_ref::<mado_pilot::Error>()
                .map(mado_pilot::Error::status);
            eprintln!(
                "ocr-text-watch: failed status={status:?}; no support or cleanup success is implied"
            );
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "ocr-text-watch-qualification")]
    let mut measurements = Measurements::start()?;

    let mut args = std::env::args_os().skip(1);
    let corpus = PathBuf::from(args.next().ok_or("a generated replay corpus is required")?);
    let scenario = args
        .next()
        .map_or(Ok("transition".to_owned()), |value| value.into_string())
        .map_err(|_| "scenario must be UTF-8")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let (target_index, output_space, width, height, sequence) = match scenario.as_str() {
        "transition" => (
            0,
            CoordinateSpace::CapturePixels,
            960.0,
            540.0,
            TRANSITION_SEQUENCE,
        ),
        "negative" => (
            1,
            CoordinateSpace::CapturePixels,
            960.0,
            540.0,
            FIRST_SOURCE_IDENTITY.1,
        ),
        "retina" => (
            2,
            CoordinateSpace::TargetLogical,
            480.0,
            270.0,
            FIRST_SOURCE_IDENTITY.1,
        ),
        _ => return Err("scenario must be transition, negative, or retina".into()),
    };
    let config = OcrProfileConfig::new(
        OcrProfile::BoundedDetector,
        controlled_path("MADO_PILOT_G004_MODEL_ROOT")?,
        controlled_path("MADO_PILOT_ONNX_RUNTIME")?,
    );
    let setup = OperationContext::new().with_timeout(Duration::from_secs(30))?;
    let engine = mado_pilot::replay_engine_with_ocr_profile(
        ReplayEngineRequest::new(ReplaySource::from_directory(&corpus)?),
        &config,
        &setup,
    )?;
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.record(Stage::EngineReady);
    let backend = engine.ocr_backend().ok_or("OCR is unavailable")?;
    let target = engine
        .discover(&setup)?
        .get(target_index)
        .ok_or("the fixed replay target is missing")?
        .id();
    let session = engine.open(target, &OpenRequest::new(), &setup)?;
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.record(Stage::SessionReady);
    let work = (|| -> Result<Arc<OcrTextTerminalOutcome>, Box<dyn std::error::Error>> {
        let query = session.start_ocr_text_watch(OcrTextWatchRequest::new(
            Rect::new(output_space, 0.0, 0.0, width, height)?,
            ClipPolicy::Reject,
            EXPECTED_TEXT[0],
            0.0,
            output_space,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_millis(1))?,
            OcrTextStability::immediate(),
            ChangeDetectionPolicy::ExactRgba,
            OperationContext::new().with_timeout(Duration::from_secs(30))?,
        )?)?;
        let _snapshot = query.poll();

        // This interruption belongs only to this wait, even if the query already finished.
        let wait_cancel = CancellationToken::new();
        wait_cancel.cancel();
        let cancelled_wait = OperationContext::new().with_cancellation(wait_cancel);
        if query
            .wait(&cancelled_wait)
            .err()
            .map(|error| error.status())
            != Some(Status::Cancelled)
        {
            return Err("independent wait authority differs".into());
        }
        let wait = OperationContext::new().with_timeout(Duration::from_secs(35))?;
        let terminal = query.wait(&wait)?;
        #[cfg(feature = "ocr-text-watch-qualification")]
        measurements.record(Stage::QueryTerminal);
        if let OcrTextTerminalOutcome::Matched(result) = terminal.as_ref() {
            if result.target() != target || result.result().backend() != &backend {
                return Err("selected target or backend correlation differs".into());
            }
            verify_result(result, sequence, output_space)?;
        }
        drop(query);
        Ok(terminal)
    })();
    let close = OperationContext::new().with_timeout(Duration::from_secs(10))?;
    let close_outcome = session.close(&close);
    #[cfg(feature = "ocr-text-watch-qualification")]
    let physical_cleanup = measurements.close_returned(&engine);
    drop(session);
    drop(engine);
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.record(Stage::ParentsDropped);
    close_outcome?;
    #[cfg(feature = "ocr-text-watch-qualification")]
    physical_cleanup?;
    let terminal = work?;

    if scenario == "negative" {
        if !matches!(terminal.as_ref(), OcrTextTerminalOutcome::SessionClosed) {
            return Err("negative source did not drain to closed".into());
        }
        println!("ocr-text-watch: scenario=negative terminal=SessionClosed cleanup=returned");
        #[cfg(feature = "ocr-text-watch-qualification")]
        {
            drop(terminal);
            measurements.finish(&scenario)?;
        }
        return Ok(());
    }
    let OcrTextTerminalOutcome::Matched(result) = terminal.as_ref() else {
        return Err("the fixed positive source did not match".into());
    };
    verify_result(result, sequence, output_space)?;
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.retained_result(result);
    let retained_frame = result.frame().clone();
    let mapping = retained_frame.map(PixelFormat::Bgra8, &close)?;
    let expected_pixels = std::fs::read(corpus.join("hud.bgra"))?;
    if mapping.stamp() != result.result().stamp() || mapping.bytes() != expected_pixels {
        return Err("retained source pixels differ after parent teardown".into());
    }
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.retained_read(&mapping)?;
    println!(
        "ocr-text-watch: scenario={scenario} terminal=Matched sequence={sequence} regions={} satisfying={} confirmations={} retained_bytes={} cleanup=returned",
        result.result().regions().len(),
        result.satisfying_region_indexes().len(),
        result.confirmed_observations(),
        mapping.bytes().len(),
    );
    // A separately retained frame remains usable after the result owner is gone too.
    drop(mapping);
    drop(terminal);
    let mapping = retained_frame.map(PixelFormat::Bgra8, &close)?;
    if mapping.bytes() != expected_pixels {
        return Err("separately retained frame differs".into());
    }
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.retained_read(&mapping)?;
    drop(mapping);
    drop(expected_pixels);
    drop(retained_frame);
    #[cfg(feature = "ocr-text-watch-qualification")]
    measurements.finish(&scenario)?;
    Ok(())
}

fn verify_result(
    result: &OcrTextWatchResult,
    sequence: u64,
    output_space: CoordinateSpace,
) -> Result<(), Box<dyn std::error::Error>> {
    let ocr = result.result();
    let stamp = result.frame().stamp();
    if stamp != ocr.stamp()
        || stamp.sequence().value() != sequence
        || stamp.epoch().value() != FIRST_SOURCE_IDENTITY.0
        || stamp.geometry().value() != FIRST_SOURCE_IDENTITY.2
        || ocr.transform() != result.frame().transform()
        || ocr.output_space() != output_space
        || ocr.effective_region() != PixelRect::new(0, 0, 960, 540)?
        || result.literal() != EXPECTED_TEXT[0]
        || result.confirmed_observations() != 1
        || result.satisfying_region_indexes() != [0]
        || ocr.backend().profile().as_str() != mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
        || result.provider().active_provider() != OcrExecutionProvider::Cpu
        || result.provider().runtime_profile().as_str()
            != mado_pilot::DEFAULT_OCR_RUNTIME_PROFILE_ID
    {
        return Err("fixed result identity or predicate facts differ".into());
    }
    if ocr.regions().len() != EXPECTED_TEXT.len()
        || ocr
            .regions()
            .iter()
            .zip(EXPECTED_TEXT)
            .any(|(region, text)| {
                region.text() != text
                    || !region.confidence().get().is_finite()
                    || !(0.0..=1.0).contains(&region.confidence().get())
                    || region
                        .geometry()
                        .points()
                        .iter()
                        .any(|point| point.space() != output_space)
            })
    {
        return Err("fixed normalized output differs".into());
    }
    if !satisfying_geometry_matches(ocr.regions()[0].geometry().points(), output_space) {
        return Err("fixed satisfying geometry differs".into());
    }
    Ok(())
}

fn satisfying_geometry_matches(
    points: [mado_pilot::Point; 4],
    output_space: CoordinateSpace,
) -> bool {
    if points.iter().any(|point| point.space() != output_space) {
        return false;
    }
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in points {
        bounds[0] = bounds[0].min(point.x());
        bounds[1] = bounds[1].min(point.y());
        bounds[2] = bounds[2].max(point.x());
        bounds[3] = bounds[3].max(point.y());
    }
    // The fixture's fixed 2x placement, not the observed transform, defines this oracle.
    let scale = if output_space == CoordinateSpace::TargetLogical {
        0.5
    } else {
        1.0
    };
    let expected = [53.0_f64, 60.0, 203.0, 112.0].map(|coordinate| coordinate * scale);
    let intersection = (bounds[2].min(expected[2]) - bounds[0].max(expected[0])).max(0.0)
        * (bounds[3].min(expected[3]) - bounds[1].max(expected[1])).max(0.0);
    let union = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]) + 150.0 * 52.0 * scale * scale
        - intersection;
    intersection / union >= 0.5
        && ((bounds[0] + bounds[2]) / 2.0 - 128.0 * scale).abs() <= 24.0 * scale
        && ((bounds[1] + bounds[3]) / 2.0 - 86.0 * scale).abs() <= 13.5 * scale
}

fn controlled_path(variable: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let value = std::env::var_os(variable).ok_or("reviewed OCR prerequisites must be explicit")?;
    Ok(PathBuf::from(value).canonicalize()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_oracle_rejects_wrong_coordinate_tags_and_unscaled_values() {
        let quad = |space, scale| {
            [(53.0, 60.0), (203.0, 60.0), (203.0, 112.0), (53.0, 112.0)]
                .map(|(x, y)| mado_pilot::Point::new(space, x * scale, y * scale).unwrap())
        };
        assert!(satisfying_geometry_matches(
            quad(CoordinateSpace::TargetLogical, 0.5),
            CoordinateSpace::TargetLogical,
        ));
        assert!(!satisfying_geometry_matches(
            quad(CoordinateSpace::CapturePixels, 1.0),
            CoordinateSpace::TargetLogical,
        ));
        assert!(!satisfying_geometry_matches(
            quad(CoordinateSpace::TargetLogical, 1.0),
            CoordinateSpace::TargetLogical,
        ));
        assert!(satisfying_geometry_matches(
            quad(CoordinateSpace::CapturePixels, 1.0),
            CoordinateSpace::CapturePixels,
        ));
    }

    #[test]
    fn fixed_source_oracle_matches_replay_publication_identity() {
        use mado_pilot::replay::{ReplayFrame, ReplayTarget};
        use mado_pilot::{
            Continuity, FrameDescriptor, FrameRequest, MonotonicInstant, PixelExtent,
        };

        let descriptor =
            FrameDescriptor::packed(PixelExtent::new(2, 2), PixelFormat::Bgra8).unwrap();
        let frames = (0_u8..2)
            .map(|fill| {
                ReplayFrame::new(
                    descriptor,
                    MonotonicInstant::from_origin(Duration::from_millis(u64::from(fill))),
                    Continuity::Continuous,
                    None,
                    vec![fill; descriptor.byte_len()].into_boxed_slice(),
                )
                .unwrap()
            })
            .collect();
        let source =
            ReplaySource::from_targets(vec![ReplayTarget::new("identity-oracle", frames).unwrap()])
                .unwrap();
        let engine = mado_pilot::replay_engine(source).unwrap();
        let operation = OperationContext::new()
            .with_timeout(Duration::from_secs(5))
            .unwrap();
        let target = engine.discover(&operation).unwrap()[0].id();
        let session = engine
            .open(target, &OpenRequest::new(), &operation)
            .unwrap();
        let first = session
            .acquire_frame(&FrameRequest::latest(), &operation)
            .unwrap()
            .stamp();
        let second = session
            .acquire_frame(&FrameRequest::newer_than(first), &operation)
            .unwrap()
            .stamp();
        session.close(&operation).unwrap();
        assert_eq!(
            (
                first.epoch().value(),
                first.sequence().value(),
                first.geometry().value()
            ),
            FIRST_SOURCE_IDENTITY
        );
        assert_eq!(second.sequence().value(), TRANSITION_SEQUENCE);
        assert_eq!(second.epoch(), first.epoch());
        assert_eq!(second.geometry(), first.geometry());
    }
}
