//! Bounded, pull-based diagnostic projections for the C boundary.
//!
//! Readers and batches own no engine resources. Facade identities carry their
//! own checked engine-local ordinals, so records remain projectable after the
//! engine handle is released without retaining a boundary identity registry.

use std::mem::size_of;

use mado_pilot::{
    DiagnosticBatch, DiagnosticDrain, DiagnosticKind, DiagnosticLevel, DiagnosticOperationKind,
    DiagnosticOptions, DiagnosticPayload, DiagnosticReader, DiagnosticRecord, InputGeometryResult,
    InputOperationKind, InputRevalidationCategory, Lifecycle, SearchDiagnosticOutcome,
};

use crate::boundary::{self, Out, Versioned, inputs, prefixes};
use crate::engine::{EngineHandle, madopilot_engine_t};
use crate::error::Fault;
use crate::handle::opaque;
use crate::input::{
    cleanup_state_code, input_address_scope_code, input_delivery_code, input_fault_code,
    input_operation_code, permission_kind_code, permission_state_code, sequence_outcome_code,
    submission_evidence_code,
};
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_ENGINE, MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK,
    madopilot_status_t,
};
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

inputs! {
    impl Input for madopilot_engine_options_t {
        const MANDATORY: usize = 16;
        const NAME: &'static str = "madopilot_engine_options_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_engine_options_t,
            struct_size,
            flags,
            diagnostic_level,
            diagnostic_capacity,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                diagnostic_level: MADOPILOT_DIAGNOSTIC_LEVEL_OFF,
                diagnostic_capacity: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
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
    const MANDATORY: usize = size_of::<madopilot_diagnostic_record_t>();
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
    );

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
        }
    }
}

/// Converts nullable C options. Null selects the allocation-free default.
pub(crate) unsafe fn options(
    options: *const madopilot_engine_options_t,
) -> Result<DiagnosticOptions, Fault> {
    if options.is_null() {
        return Ok(DiagnosticOptions::off());
    }
    // SAFETY: forwarded unchanged from this function's own contract.
    let options = unsafe { boundary::read_input(options) }?;
    if options.flags != 0 {
        return Err(Fault::abi(format!(
            "madopilot_engine_options_t sets unknown flags {:#x}",
            options.flags
        )));
    }
    let level = match options.diagnostic_level {
        MADOPILOT_DIAGNOSTIC_LEVEL_OFF => DiagnosticLevel::Off,
        MADOPILOT_DIAGNOSTIC_LEVEL_NORMAL => DiagnosticLevel::Normal,
        MADOPILOT_DIAGNOSTIC_LEVEL_DEBUG => DiagnosticLevel::Debug,
        other => return Err(Fault::abi(format!("unrecognized diagnostic level {other}"))),
    };
    DiagnosticOptions::new(level, options.diagnostic_capacity as usize)
        .map_err(|error| Fault::from_error(&error, MADOPILOT_ERROR_CATEGORY_ENGINE))
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
        DiagnosticPayload::InputEvent(payload) => project_input_event(&mut value, payload),
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
fn project_input_event(
    value: &mut madopilot_diagnostic_record_t,
    payload: mado_pilot::InputEventDiagnostic,
) {
    value.target = payload.target.get();
    value.route = input_delivery_code(payload.route);
    value.input_operations = input_operation_bit(payload.operation);
    value.requested = payload.expected_native_units;
    value.submitted = payload.invoked_native_units;
    value.cleanup_released = payload.event_index;
    value.permission_state = permission_state_code(payload.authorization);
    value.reserved = input_event_detail(payload.revalidation, payload.geometry);
    value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET
        | MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE
        | MADOPILOT_DIAGNOSTIC_RECORD_HAS_PERMISSION_STATE
        | MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_EVENT_DETAIL;
    if let Some(candidate_count) = payload.candidate_count {
        value.result_count = u64::from(candidate_count);
        value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_CANDIDATE_COUNT;
    }
    if let Some(fault) = payload.fault {
        value.input_fault = input_fault_code(fault);
        value.flags |= MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT;
    }
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
        DiagnosticKind::Input => MADOPILOT_DIAGNOSTIC_KIND_INPUT,
        DiagnosticKind::RouteAttempt => MADOPILOT_DIAGNOSTIC_KIND_ROUTE_ATTEMPT,
        DiagnosticKind::Lifecycle => MADOPILOT_DIAGNOSTIC_KIND_LIFECYCLE,
        DiagnosticKind::Permission => MADOPILOT_DIAGNOSTIC_KIND_PERMISSION,
        DiagnosticKind::InputEvent => MADOPILOT_DIAGNOSTIC_KIND_INPUT_EVENT,
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
        DiagnosticOperationKind::SessionClose => MADOPILOT_DIAGNOSTIC_OPERATION_SESSION_CLOSE,

        _ => 0,
    }
}

const fn lifecycle_code(lifecycle: Lifecycle) -> madopilot_lifecycle_t {
    match lifecycle {
        Lifecycle::Open => MADOPILOT_LIFECYCLE_OPEN,
        Lifecycle::Closing => MADOPILOT_LIFECYCLE_CLOSING,
        Lifecycle::Closed => MADOPILOT_LIFECYCLE_CLOSED,
    }
}

const fn input_operation_bit(kind: InputOperationKind) -> u32 {
    let code = input_operation_code(kind);
    if code > 0 { 1 << (code - 1) } else { 0 }
}

const fn input_event_detail(
    revalidation: InputRevalidationCategory,
    geometry: InputGeometryResult,
) -> u32 {
    let revalidation = (match revalidation {
        InputRevalidationCategory::Passed => MADOPILOT_INPUT_REVALIDATION_PASSED,
        InputRevalidationCategory::TargetLost => MADOPILOT_INPUT_REVALIDATION_TARGET_LOST,
        InputRevalidationCategory::Ambiguous => MADOPILOT_INPUT_REVALIDATION_AMBIGUOUS,
        InputRevalidationCategory::Interrupted => MADOPILOT_INPUT_REVALIDATION_INTERRUPTED,
        InputRevalidationCategory::Unavailable => MADOPILOT_INPUT_REVALIDATION_UNAVAILABLE,
        _ => 0,
    })
    .cast_unsigned();
    let geometry = (match geometry {
        InputGeometryResult::NotApplicable => MADOPILOT_INPUT_GEOMETRY_NOT_APPLICABLE,
        InputGeometryResult::Passed => MADOPILOT_INPUT_GEOMETRY_PASSED,
        InputGeometryResult::Changed => MADOPILOT_INPUT_GEOMETRY_CHANGED,
        InputGeometryResult::NotEvaluated => MADOPILOT_INPUT_GEOMETRY_NOT_EVALUATED,
        _ => 0,
    })
    .cast_unsigned();
    (revalidation & MADOPILOT_DIAGNOSTIC_INPUT_EVENT_REVALIDATION_MASK)
        | ((geometry << MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_SHIFT)
            & MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_MASK)
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use mado_pilot::{
        InputDelivery, InputEventDiagnostic, InputFault, InputGeometryResult, InputOperationKind,
        InputRevalidationCategory, PermissionState,
    };
    use mado_pilot_runtime::{IdentityIssuer, ProviderId};

    use super::{project_input_event, *};
    use crate::boundary::Versioned;

    #[test]
    fn input_event_projection_preserves_every_bounded_gate_fact_without_abi_growth() {
        let target = IdentityIssuer::new()
            .issue_target(ProviderId::new("c-diagnostic-test"))
            .expect("issued");
        let mut record = <madopilot_diagnostic_record_t as Versioned>::failure(
            u32::try_from(size_of::<madopilot_diagnostic_record_t>()).expect("record size fits"),
        );

        project_input_event(
            &mut record,
            InputEventDiagnostic {
                target,
                route: InputDelivery::ProcessDirected,
                event_index: 7,
                operation: InputOperationKind::Text,
                revalidation: InputRevalidationCategory::Ambiguous,
                candidate_count: Some(2),
                authorization: PermissionState::Granted,
                geometry: InputGeometryResult::Changed,
                expected_native_units: 4,
                invoked_native_units: 1,
                fault: Some(InputFault::GeometryChanged),
            },
        );

        assert_eq!(record.target, target.get());
        assert_eq!(record.route, MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED);
        assert_eq!(
            record.input_operations,
            1 << (MADOPILOT_INPUT_OPERATION_TEXT - 1)
        );
        assert_eq!(record.cleanup_released, 7);
        assert_eq!(record.requested, 4);
        assert_eq!(record.submitted, 1);
        assert_eq!(record.result_count, 2);
        assert_eq!(record.permission_state, MADOPILOT_PERMISSION_STATE_GRANTED);
        assert_eq!(record.input_fault, MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED);
        assert_eq!(
            record.reserved & MADOPILOT_DIAGNOSTIC_INPUT_EVENT_REVALIDATION_MASK,
            MADOPILOT_INPUT_REVALIDATION_AMBIGUOUS as u32
        );
        assert_eq!(
            (record.reserved & MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_MASK)
                >> MADOPILOT_DIAGNOSTIC_INPUT_EVENT_GEOMETRY_SHIFT,
            MADOPILOT_INPUT_GEOMETRY_CHANGED as u32
        );
        assert_eq!(
            record.flags,
            MADOPILOT_DIAGNOSTIC_RECORD_HAS_TARGET
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_ROUTE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_FAULT
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_PERMISSION_STATE
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_CANDIDATE_COUNT
                | MADOPILOT_DIAGNOSTIC_RECORD_HAS_INPUT_EVENT_DETAIL
        );
        assert_eq!(record.activity_tag, 0);
        assert_eq!(record.address_scope, MADOPILOT_INPUT_ADDRESS_NONE);
        assert_eq!(record.evidence, MADOPILOT_SUBMISSION_EVIDENCE_NONE);
        assert_eq!(record.cleanup_owed, 0);
    }
}
