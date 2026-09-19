//! Exercise real consumer decisions without constructing any engine or backend.

use std::ffi::OsString;
use std::time::Duration;

use mado_pilot::{
    CancellationToken, CleanupState, CoordinateSpace, GeometryRevision, MonotonicInstant,
    OperationContext, PixelExtent, Point, Scale, SequenceOutcome, TargetPlacement,
    TransformSnapshot,
};

use crate::config::{self, Action, KeyModifier};
use crate::decisions::{self, ReceiptFacts, SourceVersion};
use crate::{Failure, Result};

fn require(condition: bool, contract: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Failure::Policy(contract))
    }
}

pub(crate) fn run() -> Result<()> {
    require(
        crate::command(Vec::<OsString>::new()).is_err(),
        "smoke_default_refusal",
    )?;
    require(
        crate::command(["--config", "unread", "--allow-input"].map(OsString::from)).is_err(),
        "smoke_capture_consent",
    )?;
    require(
        crate::command(["--smoke", "--allow-input"].map(OsString::from)).is_err(),
        "smoke_no_authority_escalation",
    )?;
    let observe = Action::Observe;
    let click = Action::Click;
    require(
        decisions::authorize(&click, false).is_err(),
        "smoke_input_consent",
    )?;
    require(
        decisions::authorize(&observe, true).is_err(),
        "smoke_observe_separation",
    )?;
    let token = CancellationToken::new();
    let workflow = OperationContext::new()
        .with_cancellation(token.clone())
        .with_timeout(Duration::from_secs(60))
        .map_err(|error| Failure::library("smoke_deadline", error))?;
    require(
        !decisions::may_send(&observe, false, true, &workflow)?,
        "smoke_observation_cannot_send",
    )?;
    require(
        decisions::may_send(&click, true, false, &workflow).is_err(),
        "smoke_unmatched_cannot_send",
    )?;
    require(
        decisions::may_send(&click, true, true, &workflow)?,
        "smoke_authorized_match",
    )?;
    let short = decisions::bounded_child(&workflow, Duration::from_millis(50))?;
    let long = decisions::bounded_child(&workflow, Duration::from_secs(120))?;
    require(
        short.deadline() < workflow.deadline() && long.deadline() == workflow.deadline(),
        "smoke_child_deadline_cannot_extend_authority",
    )?;
    let expired_wait = short.with_deadline(MonotonicInstant::ORIGIN);
    require(
        decisions::checkpoint(&expired_wait).is_err() && decisions::checkpoint(&workflow).is_ok(),
        "smoke_independent_wait_expiry",
    )?;
    let expired_workflow = workflow.clone().with_deadline(MonotonicInstant::ORIGIN);
    require(
        decisions::may_send(&click, true, true, &expired_workflow).is_err(),
        "smoke_expired_match_cannot_send",
    )?;
    token.cancel();
    require(
        decisions::may_send(&click, true, true, &workflow).is_err()
            && decisions::checkpoint(&long).is_err(),
        "smoke_shared_cancellation",
    )?;
    require(
        decisions::unique([1, 2].into_iter()).is_err()
            && decisions::unique(std::iter::empty::<u8>()).is_err()
            && decisions::unique([7].into_iter())? == 7,
        "smoke_unique_selection",
    )?;

    let before = SourceVersion {
        epoch: 2,
        sequence: 9,
        geometry: 4,
    };
    require(
        !decisions::source_progress(true, before, before)?,
        "smoke_current_frame_is_not_postcondition",
    )?;
    require(
        decisions::source_progress(
            true,
            before,
            SourceVersion {
                sequence: 10,
                ..before
            },
        )?,
        "smoke_newer_sequence",
    )?;
    require(
        decisions::source_progress(
            true,
            before,
            SourceVersion {
                epoch: 3,
                sequence: 0,
                geometry: 5,
            },
        )?,
        "smoke_epoch_reset_is_newer",
    )?;
    require(
        decisions::source_progress(false, before, before).is_err(),
        "smoke_foreign_stream_refusal",
    )?;
    require(
        decisions::source_progress(
            true,
            before,
            SourceVersion {
                sequence: 8,
                ..before
            },
        )
        .is_err(),
        "smoke_source_regression",
    )?;
    require(
        !decisions::postcondition_progress(
            true,
            before,
            SourceVersion {
                sequence: 8,
                geometry: 3,
                ..before
            },
        )?,
        "smoke_stale_watch_match_is_rearmed",
    )?;
    require(
        decisions::postcondition_progress(false, before, before).is_err(),
        "smoke_foreign_postcondition_refusal",
    )?;
    require(
        !decisions::postcondition_progress(true, before, before)?
            && decisions::postcondition_progress(
                true,
                before,
                SourceVersion {
                    sequence: 10,
                    ..before
                },
            )?
            && decisions::postcondition_progress(
                true,
                before,
                SourceVersion {
                    geometry: 5,
                    ..before
                },
            )
            .is_err(),
        "smoke_rearmed_postcondition_accepts_only_valid_newer_source",
    )?;
    require(
        decisions::source_progress(
            true,
            before,
            SourceVersion {
                sequence: 10,
                geometry: 3,
                ..before
            },
        )
        .is_err(),
        "smoke_geometry_regression",
    )?;
    require(
        decisions::source_progress(
            true,
            before,
            SourceVersion {
                geometry: 5,
                ..before
            },
        )
        .is_err(),
        "smoke_same_source_geometry_conflict",
    )?;

    let geometry_error =
        |error: mado_pilot::GeometryFault| Failure::library("smoke_geometry", error.into());
    let placement = TargetPlacement::new(
        (300.0, 100.0),
        (200.0, 100.0),
        Scale::new(2.0, 2.0).map_err(geometry_error)?,
    )
    .map_err(geometry_error)?;
    let transform = TransformSnapshot::with_target(
        GeometryRevision::FIRST,
        PixelExtent::new(400, 200),
        placement,
    )
    .map_err(geometry_error)?;
    let points = [(100.0, 40.0), (180.0, 40.0), (180.0, 80.0), (100.0, 80.0)]
        .map(|(x, y)| Point::new(CoordinateSpace::CapturePixels, x, y).map_err(geometry_error));
    let [a, b, c, d] = points;
    let points = [a?, b?, c?, d?];
    let center = decisions::region_center(points, &transform)?;
    require(
        center.space() == CoordinateSpace::TargetLogical
            && center.x() == 70.0
            && center.y() == 30.0,
        "smoke_same_source_retina_conversion",
    )?;
    let mut mixed = points;
    mixed[0] = Point::new(CoordinateSpace::TargetLogical, 100.0, 40.0).map_err(geometry_error)?;
    require(
        decisions::region_center(mixed, &transform).is_err(),
        "smoke_mixed_coordinate_refusal",
    )?;
    let no_placement =
        TransformSnapshot::frame_only(GeometryRevision::FIRST, PixelExtent::new(400, 200));
    require(
        decisions::region_center(points, &no_placement).is_err(),
        "smoke_no_invented_target_transform",
    )?;
    let click_sequence = decisions::recipe(&click, Some(center))?;
    require(
        click_sequence.held_after(click_sequence.len()).is_empty()
            && click_sequence.held_after(2).len() == 1,
        "smoke_balanced_click_and_cleanup",
    )?;
    let chord = Action::Chord {
        key: "Enter".into(),
        modifiers: vec![KeyModifier::Control, KeyModifier::Shift],
    };
    let chord_sequence = decisions::recipe(&chord, None)?;
    require(
        chord_sequence.held_after(chord_sequence.len()).is_empty()
            && chord_sequence.possibly_held_after(2, true).len() == 3,
        "smoke_balanced_chord_partial_press_cleanup",
    )?;
    let text_sequence = decisions::recipe(
        &Action::Text {
            text: "model-free".into(),
        },
        None,
    )?;
    require(
        text_sequence.held_after(text_sequence.len()).is_empty(),
        "smoke_text_owns_no_pressed_state",
    )?;
    require(
        decisions::recipe(&observe, None).is_err() && decisions::recipe(&click, None).is_err(),
        "smoke_no_unbound_action",
    )?;

    let complete = ReceiptFacts {
        outcome: SequenceOutcome::Complete,
        submitted: 3,
        partial_native_effect: false,
        fault: false,
        cleanup: CleanupState::NotNeeded,
        released: 0,
        owed: 0,
    };
    require(
        complete.submission_complete() && complete.should_observe(),
        "smoke_complete_is_submission_only",
    )?;
    let partial = ReceiptFacts {
        outcome: SequenceOutcome::Partial,
        submitted: 0,
        partial_native_effect: true,
        fault: true,
        cleanup: CleanupState::Exhausted,
        released: 1,
        owed: 2,
    };
    require(
        !partial.submission_complete() && partial.should_observe(),
        "smoke_partial_first_event_is_possible_effect",
    )?;
    let cleaned_partial = ReceiptFacts {
        cleanup: CleanupState::Complete,
        released: 2,
        ..partial
    };
    require(
        !cleaned_partial.submission_complete() && cleaned_partial.should_observe(),
        "smoke_cleanup_cannot_make_partial_complete",
    )?;
    let incomplete_cleanup = ReceiptFacts {
        cleanup: CleanupState::Incomplete,
        released: 1,
        owed: 2,
        ..complete
    };
    require(
        !incomplete_cleanup.submission_complete(),
        "smoke_incomplete_cleanup_not_success",
    )?;
    let unexecuted = ReceiptFacts {
        outcome: SequenceOutcome::Unexecuted,
        submitted: 0,
        partial_native_effect: false,
        fault: true,
        ..complete
    };
    require(
        !unexecuted.submission_complete() && !unexecuted.should_observe(),
        "smoke_unexecuted_no_application_effect_inference",
    )?;

    config_boundaries()?;
    Ok(())
}

fn config_boundaries() -> Result<()> {
    // Paths are parsed, never opened; this is not an operational sample or fixture.
    let root = if cfg!(windows) {
        "C:\\unopened\\models"
    } else {
        "/unopened/models"
    };
    let runtime = if cfg!(windows) {
        "C:\\unopened\\runtime.dll"
    } else {
        "/unopened/runtime.dylib"
    };
    let mut value = serde_json::json!({
        "target": { "window_title": "unopened" },
        "ocr": { "model_root": root, "runtime_path": runtime },
        "before": { "roi": { "x": 20, "y": 30, "width": 40, "height": 50 },
            "literal": "unobserved", "minimum_confidence": 0.9 },
        "action": { "kind": "observe" },
        "budgets": { "workflow_ms": 1000, "wait_ms": 100, "postcondition_ms": 100, "close_ms": 100 },
        "analysis_interval_ms": 50
    });
    let encode = |value: &serde_json::Value| {
        serde_json::to_vec(value).map_err(|_| Failure::Policy("smoke_json_encoding"))
    };
    let config = config::decode(&encode(&value)?)?;
    let roi = config.before.roi.rect()?;
    require(
        roi.left() == 20.0 && roi.top() == 30.0 && roi.right() == 60.0 && roi.bottom() == 80.0,
        "smoke_roi_size_is_not_far_edge",
    )?;
    value["before"]["unexpected"] = serde_json::json!(true);
    require(
        config::decode(&encode(&value)?).is_err(),
        "smoke_unknown_fields_refused",
    )?;
    require(
        config::decode(&vec![b' '; config::MAX_CONFIG_BYTES + 1]).is_err(),
        "smoke_config_size_limit",
    )?;
    Ok(())
}
