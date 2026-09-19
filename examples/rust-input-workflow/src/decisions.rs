//! Pure policy shared by operational use and the model-free smoke procedure.

use std::cmp::Ordering;
use std::time::Duration;

use mado_pilot::{
    CleanupState, CoordinateSpace, FrameStamp, InputEvent, InputReceipt, InputSequence,
    OcrTextWatchResult, OperationContext, Point, PointerButton, SequenceLimits, SequenceOutcome,
    TransformSnapshot,
};

use crate::config::{Action, parse_key};
use crate::{Failure, Result};

pub(crate) fn authorize(action: &Action, allow_input: bool) -> Result<()> {
    match (action.operation().is_some(), allow_input) {
        (true, false) => Err(Failure::Policy("separate_input_consent_required")),
        (false, true) => Err(Failure::Policy("observe_must_not_request_input_authority")),
        _ => Ok(()),
    }
}

pub(crate) fn checkpoint(operation: &OperationContext) -> Result<()> {
    if let Some(interruption) = operation.interruption() {
        return Err(Failure::library("workflow_authority", interruption.into()));
    }
    Ok(())
}

pub(crate) fn may_send(
    action: &Action,
    allow_input: bool,
    matched: bool,
    operation: &OperationContext,
) -> Result<bool> {
    authorize(action, allow_input)?;
    checkpoint(operation)?;
    if action.operation().is_none() {
        return Ok(false);
    }
    if !matched {
        return Err(Failure::Policy("accepted_observation_required"));
    }
    Ok(true)
}

/// A child wait has its own deadline, never later than the workflow's absolute one.
pub(crate) fn bounded_child(
    parent: &OperationContext,
    timeout: Duration,
) -> Result<OperationContext> {
    checkpoint(parent)?;
    let deadline = parent
        .deadline()
        .ok_or(Failure::Policy("finite_parent_deadline_required"))?;
    let local = parent
        .now()
        .checked_add(timeout)
        .ok_or(Failure::Policy("child_deadline_overflow"))?;
    Ok(parent.clone().with_deadline(deadline.min(local)))
}

pub(crate) fn unique<T>(mut matches: impl Iterator<Item = T>) -> Result<T> {
    let selected = matches.next().ok_or(Failure::Policy("target_missing"))?;
    if matches.next().is_some() {
        return Err(Failure::Policy("target_ambiguous"));
    }
    Ok(selected)
}

#[derive(Clone, Copy)]
pub(crate) struct SourceVersion {
    pub epoch: u64,
    pub sequence: u64,
    pub geometry: u64,
}

impl From<FrameStamp> for SourceVersion {
    fn from(stamp: FrameStamp) -> Self {
        Self {
            epoch: stamp.epoch().value(),
            sequence: stamp.sequence().value(),
            geometry: stamp.geometry().value(),
        }
    }
}

/// Numeric components never replace the engine-qualified stream comparison.
pub(crate) fn source_progress(
    same_stream: bool,
    old: SourceVersion,
    new: SourceVersion,
) -> Result<bool> {
    if !same_stream {
        return Err(Failure::Policy("foreign_source_stream"));
    }
    if new.geometry < old.geometry {
        return Err(Failure::Policy("source_geometry_regression"));
    }
    match (new.epoch, new.sequence).cmp(&(old.epoch, old.sequence)) {
        Ordering::Less => Err(Failure::Policy("source_order_regression")),
        Ordering::Equal if new.geometry != old.geometry => {
            Err(Failure::Policy("same_source_geometry_disagrees"))
        }
        Ordering::Equal => Ok(false),
        Ordering::Greater => Ok(true),
    }
}

pub(crate) fn newer(old: FrameStamp, new: FrameStamp) -> Result<bool> {
    source_progress(old.is_same_stream(&new), old.into(), new.into())
}

pub(crate) fn postcondition_progress(
    same_stream: bool,
    checkpoint: SourceVersion,
    source: SourceVersion,
) -> Result<bool> {
    if !same_stream {
        return Err(Failure::Policy("foreign_source_stream"));
    }
    // An in-flight watcher acquisition may publish an older frame to a new query.
    if (source.epoch, source.sequence) < (checkpoint.epoch, checkpoint.sequence) {
        return Ok(false);
    }
    source_progress(true, checkpoint, source)
}

pub(crate) fn require_action_geometry(old: FrameStamp, current: FrameStamp) -> Result<()> {
    newer(old, current)?;
    if old.epoch() != current.epoch() || old.geometry() != current.geometry() {
        return Err(Failure::Policy("observation_epoch_or_geometry_changed"));
    }
    Ok(())
}

/// Use the convex quadrilateral's vertex mean, in its own source space, then
/// convert with that same retained frame. No desktop scale or newest transform.
pub(crate) fn region_center(points: [Point; 4], transform: &TransformSnapshot) -> Result<Point> {
    let space = points[0].space();
    if points.iter().any(|point| point.space() != space) {
        return Err(Failure::Policy("region_coordinate_spaces_disagree"));
    }
    let center = Point::new(
        space,
        points.iter().map(|p| p.x() / 4.0).sum(),
        points.iter().map(|p| p.y() / 4.0).sum(),
    )
    .map_err(|error| Failure::library("region_center", error.into()))?;
    let pixel = transform
        .convert_point(center, CoordinateSpace::CapturePixels)
        .map_err(|error| Failure::library("region_capture_transform", error.into()))?;
    let extent = transform.frame_extent();
    if pixel.x() < 0.0
        || pixel.y() < 0.0
        || pixel.x() >= f64::from(extent.width())
        || pixel.y() >= f64::from(extent.height())
    {
        return Err(Failure::Policy("region_center_outside_source"));
    }
    transform
        .convert_point(pixel, CoordinateSpace::TargetLogical)
        .map_err(|error| Failure::library("region_target_transform", error.into()))
}

pub(crate) fn click_point(result: &OcrTextWatchResult) -> Result<Point> {
    let ocr = result.result();
    if ocr.stamp() != result.frame().stamp() || ocr.transform() != result.frame().transform() {
        return Err(Failure::Policy("retained_result_source_disagrees"));
    }
    let [index] = result.satisfying_region_indexes() else {
        return Err(Failure::Policy("click_requires_one_satisfying_region"));
    };
    let region = ocr
        .regions()
        .get(usize::from(*index))
        .ok_or(Failure::Policy("satisfying_region_index"))?;
    region_center(region.geometry().points(), ocr.transform())
}

pub(crate) fn recipe(action: &Action, click: Option<Point>) -> Result<InputSequence> {
    let events = match action {
        Action::Observe => return Err(Failure::Policy("observe_has_no_recipe")),
        Action::Click => vec![
            InputEvent::PointerMove(click.ok_or(Failure::Policy("click_source_required"))?),
            InputEvent::PointerPress(PointerButton::Primary),
            InputEvent::PointerRelease(PointerButton::Primary),
        ],
        Action::Text { text } => vec![InputEvent::Text(text.clone())],
        Action::Chord { key, modifiers } => {
            let key = parse_key(key)?;
            let mut events = Vec::with_capacity(2 * (modifiers.len() + 1));
            events.extend(
                modifiers
                    .iter()
                    .map(|modifier| InputEvent::KeyPress(modifier.key())),
            );
            events.push(InputEvent::KeyPress(key));
            events.push(InputEvent::KeyRelease(key));
            events.extend(
                modifiers
                    .iter()
                    .rev()
                    .map(|modifier| InputEvent::KeyRelease(modifier.key())),
            );
            events
        }
    };
    let sequence = InputSequence::within(events, SequenceLimits::at_most(10)).map_err(|fault| {
        Failure::Input {
            stage: "recipe",
            fault,
        }
    })?;
    if !sequence.held_after(sequence.len()).is_empty() {
        return Err(Failure::Policy("recipe_leaves_pressed_state"));
    }
    Ok(sequence)
}

#[derive(Clone, Copy)]
pub(crate) struct ReceiptFacts {
    pub outcome: SequenceOutcome,
    pub submitted: usize,
    pub partial_native_effect: bool,
    pub fault: bool,
    pub cleanup: CleanupState,
    pub released: usize,
    pub owed: usize,
}

impl From<&InputReceipt> for ReceiptFacts {
    fn from(receipt: &InputReceipt) -> Self {
        Self {
            outcome: receipt.outcome(),
            submitted: receipt.submitted(),
            partial_native_effect: receipt.partial_native_effect(),
            fault: receipt.fault().is_some(),
            cleanup: receipt.cleanup(),
            released: receipt.cleanup_released(),
            owed: receipt.cleanup_owed(),
        }
    }
}

impl ReceiptFacts {
    /// Observation is meaningful after any possible effect, including zero full events.
    /// There is intentionally no retry/fallback decision in this consumer.
    pub fn should_observe(self) -> bool {
        self.submitted != 0 || self.partial_native_effect
    }

    pub fn submission_complete(self) -> bool {
        self.outcome == SequenceOutcome::Complete
            && self.submitted != 0
            && !self.partial_native_effect
            && !self.fault
            && matches!(
                self.cleanup,
                CleanupState::NotNeeded | CleanupState::Complete
            )
            && self.released == self.owed
    }
}
