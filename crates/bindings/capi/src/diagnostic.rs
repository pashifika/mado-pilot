//! Bounded, pull-based diagnostic projections for the C boundary.
//!
//! Readers and batches own no engine resources. Facade identities carry their
//! own checked engine-local ordinals, so records remain projectable after the
//! engine handle is released without retaining a boundary identity registry.

use std::mem::size_of;

use mado_pilot::{
    ClipPolicy, DiagnosticBatch, DiagnosticDrain, DiagnosticKind, DiagnosticLevel,
    DiagnosticOperationKind, DiagnosticPayload, DiagnosticReader, DiagnosticRecord, Lifecycle,
    OcrDiagnosticOutcome, OcrDiagnosticProfile, SearchDiagnosticOutcome,
};

use crate::boundary::{self, Out, Versioned, covers, prefixes};
use crate::engine::{EngineHandle, madopilot_engine_t};
use crate::error::Fault;
use crate::handle::opaque;
use crate::input::{
    cleanup_state_code, input_address_scope_code, input_delivery_code, input_fault_code,
    permission_kind_code, permission_state_code, sequence_outcome_code, submission_evidence_code,
};
use crate::status::{MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t};
use crate::types::*;
use crate::{handle, hooks};

opaque! {
    /// The one independently owned pull reader for an enabled engine.
    madopilot_diagnostic_reader_t => DiagnosticReaderHandle
}

opaque! {
    /// One immutable owned diagnostic batch.
    madopilot_diagnostic_batch_t => DiagnosticBatchHandle
}

#[derive(Debug)]
pub(crate) struct DiagnosticReaderHandle {
    reader: DiagnosticReader,
}

#[derive(Debug)]
pub(crate) struct DiagnosticBatchHandle {
    batch: DiagnosticBatch,
}

impl Versioned for madopilot_diagnostic_batch_info_t {
    const MANDATORY: usize = 32;
    const NAME: &'static str = "madopilot_diagnostic_batch_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_diagnostic_batch_info_t,
        struct_size,
        flags,
        record_count,
        discarded_normal,
        discarded_debug,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            record_count: 0,
            discarded_normal: 0,
            discarded_debug: 0,
        }
    }
}

impl Versioned for madopilot_diagnostic_record_t {
    const MANDATORY: usize = 240;
    const NAME: &'static str = "madopilot_diagnostic_record_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_diagnostic_record_t,
        struct_size,
        flags,
        sequence,
        timestamp_nanos,
        operation_id,
        activity_tag,
        level,
        kind,
        operation,
        status,
        target,
        frame,
        template_identity,
        source_space,
        destination_space,
        region,
        route,
        address_scope,
        evidence,
        input_fault,
        input_outcome,
        cleanup,
        permission_kind,
        permission_state,
        lifecycle,
        search_outcome,
        input_operations,
        partial_native_effect,
        used_fallback,
        reserved,
        requested,
        submitted,
        result_count,
        cleanup_released,
        cleanup_owed,
        ocr_model_instance,
        ocr_profile,
        ocr_outcome,
        ocr_requested_region,
        ocr_elapsed_nanos,
        ocr_source_pixels,
        ocr_source_envelope,
        ocr_grouped_reserved,
        ocr_zone_count,
        ocr_unique_candidate_count,
        ocr_membership_count,
        ocr_result_bytes,
        ocr_detector_runs,
        ocr_recognizer_runs,
        ocr_detector_bytes,
        ocr_recognizer_bytes,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[(
        covers!(madopilot_diagnostic_record_t, reserved: u32),
        std::mem::offset_of!(madopilot_diagnostic_record_t, requested),
    )];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            sequence: 0,
            timestamp_nanos: 0,
            operation_id: 0,
            activity_tag: 0,
            level: MADOPILOT_DIAGNOSTIC_LEVEL_OFF,
            kind: 0,
            operation: 0,
            status: MADOPILOT_STATUS_OK,
            target: 0,
            frame: madopilot_frame_stamp_t::cleared(
                u32::try_from(size_of::<madopilot_frame_stamp_t>())
                    .expect("the frame-stamp structure fits uint32_t"),
            ),
            template_identity: 0,
            source_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            destination_space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            region: madopilot_pixel_rect_t::empty(),
            route: MADOPILOT_INPUT_DELIVERY_NONE,
            address_scope: MADOPILOT_INPUT_ADDRESS_NONE,
            evidence: MADOPILOT_SUBMISSION_EVIDENCE_NONE,
            input_fault: MADOPILOT_INPUT_FAULT_NONE,
            input_outcome: MADOPILOT_SEQUENCE_UNEXECUTED,
            cleanup: MADOPILOT_CLEANUP_NOT_NEEDED,
            permission_kind: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            permission_state: MADOPILOT_PERMISSION_STATE_UNKNOWN,
            lifecycle: 0,
            search_outcome: 0,
            input_operations: 0,
            partial_native_effect: 0,
            used_fallback: 0,
            reserved: 0,
            requested: 0,
            submitted: 0,
            result_count: 0,
            cleanup_released: 0,
            cleanup_owed: 0,
            ocr_model_instance: 0,
            ocr_profile: MADOPILOT_OCR_DIAGNOSTIC_PROFILE_UNSPECIFIED,
            ocr_outcome: MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_UNSPECIFIED,
            ocr_requested_region: madopilot_ocr_requested_region_t::empty(),
            ocr_elapsed_nanos: 0,
            ocr_source_pixels: 0,
            ocr_source_envelope: madopilot_pixel_rect_t::empty(),
            ocr_grouped_reserved: 0,
            ocr_zone_count: 0,
            ocr_unique_candidate_count: 0,
            ocr_membership_count: 0,
            ocr_result_bytes: 0,
            ocr_detector_runs: 0,
            ocr_recognizer_runs: 0,
            ocr_detector_bytes: 0,
            ocr_recognizer_bytes: 0,
        }
    }
}

pub(crate) fn take_reader(
    engine: *const madopilot_engine_t,
    out_reader: *mut *mut madopilot_diagnostic_reader_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { boundary::begin_handle_out(out_reader, "out_reader") } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    if let Some(reader) = engine.take_diagnostic_reader() {
        let reader = DiagnosticReaderHandle { reader };
        // SAFETY: `out_reader` was validated above.
        unsafe { out_reader.write(handle::into_raw(reader)) };
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn reader_retain(reader: *const madopilot_diagnostic_reader_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<DiagnosticReaderHandle>(reader) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn reader_release(reader: *mut madopilot_diagnostic_reader_t) -> madopilot_status_t {
    // SAFETY: as `reader_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<DiagnosticReaderHandle>(reader) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn reader_drain(
    reader: *const madopilot_diagnostic_reader_t,
    out_state: *mut madopilot_diagnostic_drain_state_t,
    out_batch: *mut *mut madopilot_diagnostic_batch_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    let state = unsafe {
        boundary::begin_scalar_out(
            out_state,
            "out_state",
            MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY,
        )
    };
    // SAFETY: as above.
    let batch = unsafe { boundary::begin_handle_out(out_batch, "out_batch") };
    if let Err(fault) = state.and(batch) {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the reader retained for the call.
    let Some(reader) = (unsafe { handle::borrow::<DiagnosticReaderHandle>(reader) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let state = match reader.reader.drain() {
        DiagnosticDrain::Batch(batch) => {
            let batch = DiagnosticBatchHandle { batch };
            // SAFETY: `out_batch` was validated above.
            unsafe { out_batch.write(handle::into_raw(batch)) };
            MADOPILOT_DIAGNOSTIC_DRAIN_BATCH
        }
        DiagnosticDrain::OpenEmpty => MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY,
        DiagnosticDrain::EndOfStream => MADOPILOT_DIAGNOSTIC_DRAIN_END_OF_STREAM,
        _ => MADOPILOT_DIAGNOSTIC_DRAIN_OPEN_EMPTY,
    };
    // SAFETY: `out_state` was validated above.
    unsafe { boundary::commit_scalar(out_state, state) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn batch_retain(batch: *const madopilot_diagnostic_batch_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<DiagnosticBatchHandle>(batch) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn batch_release(batch: *mut madopilot_diagnostic_batch_t) -> madopilot_status_t {
    // SAFETY: as `batch_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<DiagnosticBatchHandle>(batch) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn batch_info(
    batch: *const madopilot_diagnostic_batch_t,
    out_info: *mut madopilot_diagnostic_batch_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the batch retained for the call.
    let Some(batch) = (unsafe { handle::borrow::<DiagnosticBatchHandle>(batch) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let record_count = match u64::try_from(batch.batch.len()) {
        Ok(count) => count,
        Err(_) => return crate::status::MADOPILOT_STATUS_INTERNAL,
    };
    let losses = batch.batch.losses();
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_diagnostic_batch_info_t {
            struct_size: out.declared_size(),
            flags: 0,
            record_count,
            discarded_normal: losses.normal(),
            discarded_debug: losses.debug(),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn batch_record_at(
    batch: *const madopilot_diagnostic_batch_t,
    index: usize,
    out_record: *mut madopilot_diagnostic_record_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_record) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the batch retained for the call.
    let Some(batch) = (unsafe { handle::borrow::<DiagnosticBatchHandle>(batch) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let index = match boundary::index_within(index, batch.batch.records().len(), "record") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let value = match record(&batch.batch.records()[index], out.declared_size()) {
        Ok(value) => value,
        Err(fault) => return fault.status(),
    };
    // SAFETY: `out` was validated above.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

fn record(
    record: &DiagnosticRecord,
    struct_size: u32,
) -> Result<madopilot_diagnostic_record_t, Fault> {
    let mut value = madopilot_diagnostic_record_t::failure(struct_size);
    value.sequence = record.sequence().get();
    value.timestamp_nanos =
        u64::try_from(record.timestamp().since_origin().as_nanos()).unwrap_or(u64::MAX);
    value.operation_id = record.operation().get();
    value.level = diagnostic_level_code(record.level());
    value.kind = diagnostic_kind_code(record.kind());
    if let Some(activity) = record.activity() {
        value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_ACTIVITY;
        value.activity_tag = activity.get();
    }

    match record.payload() {
        DiagnosticPayload::OperationStarted(payload) => {
            value.operation = diagnostic_operation_code(payload.operation);
        }
        DiagnosticPayload::Frame(payload) => {
            value.target = payload.target.get();
            value.frame = boundary_frame(payload.frame);
            value.flags |=
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET | MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME;
        }
        DiagnosticPayload::Mapping(payload) => {
            value.target = payload.target.get();
            value.frame = boundary_frame(payload.frame);
            value.source_space = space_code(payload.source);
            value.destination_space = space_code(payload.destination);
            value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE;
        }
        DiagnosticPayload::Search(payload) => {
            value.target = payload.target.get();
            value.flags |=
                MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET | MADOPILOT_DIAGNOSTIC_RECORD_HAS_TEMPLATE;
            if let Some(frame) = payload.frame {
                value.frame = boundary_frame(frame);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME;
            }
            value.template_identity = payload.template.get();
            if let Some(region) = payload.region {
                value.region = crate::capture::rect(region);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION;
            }
            value.result_count = payload.result_count;
            match payload.outcome {
                SearchDiagnosticOutcome::Matched => {
                    value.search_outcome = MADOPILOT_SEARCH_DIAGNOSTIC_MATCHED;
                }
                SearchDiagnosticOutcome::NoMatch => {
                    value.search_outcome = MADOPILOT_SEARCH_DIAGNOSTIC_NO_MATCH;
                }
                SearchDiagnosticOutcome::Failed(status) => {
                    value.search_outcome = MADOPILOT_SEARCH_DIAGNOSTIC_FAILED;
                    value.status = crate::status::code(status);
                    value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS;
                }
                _ => {
                    return Err(Fault::internal(
                        "a search diagnostic outcome has no C ABI projection",
                    ));
                }
            }
        }
        DiagnosticPayload::Ocr(payload) => {
            value.frame = boundary_frame(payload.source);
            value.source_space = payload
                .requested_region
                .map_or(MADOPILOT_SPACE_CAPTURE_PIXELS, |region| {
                    space_code(region.space())
                });
            value.destination_space = space_code(payload.output_space);
            value.result_count = payload.result_count;
            value.ocr_model_instance = payload.model_instance.get();
            value.ocr_elapsed_nanos = payload.elapsed_nanos;
            value.ocr_source_pixels = payload.source_pixels;
            value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_FRAME
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_SOURCE_SPACE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_DESTINATION_SPACE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_MODEL_INSTANCE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_TIMING
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESOURCES;
            match payload.profile {
                OcrDiagnosticProfile::AcceptedG004 => {
                    value.ocr_profile = MADOPILOT_OCR_DIAGNOSTIC_PROFILE_G004;
                    value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE;
                }
                OcrDiagnosticProfile::BoundedDetector => {
                    value.ocr_profile = MADOPILOT_OCR_DIAGNOSTIC_PROFILE_BOUNDED;
                    value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_PROFILE;
                }
                OcrDiagnosticProfile::Unspecified => {}
                _ => {
                    return Err(Fault::internal(
                        "an OCR diagnostic profile has no C ABI projection",
                    ));
                }
            }
            if let Some(requested) = payload.requested_region {
                value.ocr_requested_region = madopilot_ocr_requested_region_t {
                    space: space_code(requested.space()),
                    clip_policy: clip_policy_code(requested.clip_policy),
                    left: requested.left(),
                    top: requested.top(),
                    right: requested.right(),
                    bottom: requested.bottom(),
                };
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_REQUESTED_REGION;
            }
            if let Some(effective) = payload.effective_region {
                value.region = crate::capture::rect(effective);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_REGION;
            }
            if let Some(envelope) = payload.source_envelope {
                value.ocr_source_envelope = crate::capture::rect(envelope);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_SOURCE_ENVELOPE;
            }
            if let Some(zone_count) = payload.zone_count {
                value.ocr_zone_count = zone_count;
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_ZONE_COUNT;
            }
            if let (Some(unique), Some(memberships)) =
                (payload.unique_candidate_count, payload.membership_count)
            {
                value.ocr_unique_candidate_count = unique;
                value.ocr_membership_count = memberships;
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_COUNTS;
            }
            if let Some(result_bytes) = payload.result_bytes {
                value.ocr_result_bytes = result_bytes;
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_RESULT_BYTES;
            }
            if let (
                Some(detector_runs),
                Some(detector_bytes),
                Some(recognizer_runs),
                Some(recognizer_bytes),
            ) = (
                payload.detector_runs,
                payload.detector_bytes,
                payload.recognizer_runs,
                payload.recognizer_bytes,
            ) {
                value.ocr_detector_runs = detector_runs;
                value.ocr_detector_bytes = detector_bytes;
                value.ocr_recognizer_runs = recognizer_runs;
                value.ocr_recognizer_bytes = recognizer_bytes;
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_OCR_BACKEND_WORK;
            }
            match payload.outcome {
                OcrDiagnosticOutcome::Recognized => {
                    value.ocr_outcome = MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_RECOGNIZED;
                }
                OcrDiagnosticOutcome::Empty => {
                    value.ocr_outcome = MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_EMPTY;
                }
                OcrDiagnosticOutcome::Failed(status) => {
                    value.ocr_outcome = MADOPILOT_OCR_DIAGNOSTIC_OUTCOME_FAILED;
                    value.status = crate::status::code(status);
                    value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS;
                }
                _ => {
                    return Err(Fault::internal(
                        "an OCR diagnostic outcome has no C ABI projection",
                    ));
                }
            }
        }
        DiagnosticPayload::Input(payload) => {
            value.target = payload.target.get();
            value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET;
            value.input_operations = payload.operations.bits() as u32;
            value.requested = payload.requested;
            value.input_outcome = sequence_outcome_code(payload.outcome);
            value.submitted = payload.submitted;
            value.partial_native_effect = i32::from(payload.partial_native_effect);
            value.used_fallback = i32::from(payload.fallback);
            value.cleanup = cleanup_state_code(payload.cleanup);
            value.cleanup_released = payload.cleanup_released;
            value.cleanup_owed = payload.cleanup_owed;
            if let Some(route) = payload.route {
                value.route = input_delivery_code(route);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE;
            }
            if let Some(scope) = payload.address_scope {
                value.address_scope = input_address_scope_code(scope);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_ADDRESS_SCOPE;
            }
            if let Some(evidence) = payload.evidence {
                value.evidence = submission_evidence_code(evidence);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_EVIDENCE;
            }
            if let Some(fault) = payload.fault {
                value.input_fault = input_fault_code(fault);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT;
            }
            if let Some(status) = payload.status {
                value.status = crate::status::code(status);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS;
            }
        }
        DiagnosticPayload::RouteAttempt(payload) => {
            value.target = payload.target.get();
            value.route = input_delivery_code(payload.route);
            value.address_scope = input_address_scope_code(payload.address_scope);
            value.input_outcome = sequence_outcome_code(payload.outcome);
            value.submitted = payload.submitted;
            value.partial_native_effect = i32::from(payload.partial_native_effect);
            value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_ADDRESS_SCOPE;
            if let Some(evidence) = payload.evidence {
                value.evidence = submission_evidence_code(evidence);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_EVIDENCE;
            }
            if let Some(fault) = payload.fault {
                value.input_fault = input_fault_code(fault);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT;
            }
        }
        DiagnosticPayload::Lifecycle(payload) => {
            value.lifecycle = lifecycle_code(payload.lifecycle);
            if let Some(target) = payload.target {
                value.target = target.get();
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET;
            }
            if let Some(status) = payload.fault {
                value.status = crate::status::code(status);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS;
            }
        }
        DiagnosticPayload::Permission(payload) => {
            value.permission_kind = permission_kind_code(payload.permission);
            if let Some(state) = payload.state {
                value.permission_state = permission_state_code(state);

                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_PERMISSION_STATE;
            }
            if let Some(status) = payload.fault {
                value.status = crate::status::code(status);
                value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_STATUS;
            }
        }
        _ => {
            return Err(Fault::internal(
                "a diagnostic payload has no C ABI projection",
            ));
        }
    }
    Ok(value)
}

fn boundary_frame(frame: mado_pilot::FrameStamp) -> madopilot_frame_stamp_t {
    crate::capture::stamp(
        frame,
        u32::try_from(size_of::<madopilot_frame_stamp_t>())
            .expect("the frame-stamp structure fits uint32_t"),
    )
}

const fn diagnostic_level_code(level: DiagnosticLevel) -> madopilot_diagnostic_level_t {
    match level {
        DiagnosticLevel::Off => MADOPILOT_DIAGNOSTIC_LEVEL_OFF,
        DiagnosticLevel::Normal => MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL,
        DiagnosticLevel::Debug => MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG,
        _ => 0,
    }
}

const fn diagnostic_kind_code(kind: DiagnosticKind) -> madopilot_diagnostic_kind_t {
    match kind {
        DiagnosticKind::OperationStarted => MADOPILOT_DIAGNOSTIC_KIND_OPERATION_STARTED,
        DiagnosticKind::Frame => MADOPILOT_DIAGNOSTIC_KIND_FRAME,
        DiagnosticKind::Mapping => MADOPILOT_DIAGNOSTIC_KIND_MAPPING,
        DiagnosticKind::Search => MADOPILOT_DIAGNOSTIC_KIND_SEARCH,
        DiagnosticKind::Ocr => MADOPILOT_DIAGNOSTIC_KIND_OCR,
        DiagnosticKind::Input => MADOPILOT_DIAGNOSTIC_KIND_INPUT,
        DiagnosticKind::RouteAttempt => MADOPILOT_DIAGNOSTIC_KIND_ROUTE_ATTEMPT,
        DiagnosticKind::Lifecycle => MADOPILOT_DIAGNOSTIC_KIND_LIFECYCLE,
        DiagnosticKind::Permission => MADOPILOT_DIAGNOSTIC_KIND_PERMISSION,
        _ => 0,
    }
}

const fn diagnostic_operation_code(
    operation: DiagnosticOperationKind,
) -> madopilot_diagnostic_operation_kind_t {
    match operation {
        DiagnosticOperationKind::Discovery => MADOPILOT_DIAGNOSTIC_OPERATION_DISCOVERY,
        DiagnosticOperationKind::InputDescription => {
            MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_DESCRIPTION
        }
        DiagnosticOperationKind::Permission => MADOPILOT_DIAGNOSTIC_OPERATION_PERMISSION,
        DiagnosticOperationKind::SessionOpen => MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_OPEN,
        DiagnosticOperationKind::FrameAcquire => MADOPILOT_DIAGNOSTIC_OPERATION_FRAME_ACQUIRE,
        DiagnosticOperationKind::Mapping => MADOPILOT_DIAGNOSTIC_OPERATION_MAPPING,
        DiagnosticOperationKind::TemplatePreparation => {
            MADOPILOT_DIAGNOSTIC_OPERATION_TEMPLATE_PREPARATION
        }
        DiagnosticOperationKind::Search => MADOPILOT_DIAGNOSTIC_OPERATION_SEARCH,
        DiagnosticOperationKind::InputSubmission => MADOPILOT_DIAGNOSTIC_OPERATION_INPUT_SUBMISSION,
        DiagnosticOperationKind::OcrRecognition => MADOPILOT_DIAGNOSTIC_OPERATION_OCR_RECOGNITION,
        DiagnosticOperationKind::SessionClose => MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_CLOSE,

        _ => 0,
    }
}

const fn clip_policy_code(policy: ClipPolicy) -> madopilot_clip_policy_t {
    match policy {
        ClipPolicy::Reject => MADOPILOT_CLIP_POLICY_REJECT,
        ClipPolicy::Clip => MADOPILOT_CLIP_POLICY_CLIP,
    }
}

const fn lifecycle_code(lifecycle: Lifecycle) -> madopilot_lifecycle_t {
    match lifecycle {
        Lifecycle::Open => MADOPILOT_LIFECYCLE_OPEN,
        Lifecycle::Closing => MADOPILOT_LIFECYCLE_CLOSING,
        Lifecycle::Closed => MADOPILOT_LIFECYCLE_CLOSED,
    }
}
