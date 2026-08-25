//! Native capability and input projections for the C boundary.
//!
//! The facade owns admission and delivery semantics. This module validates the C
//! representation, converts it into that vocabulary, and projects immutable
//! answers into size-versioned records and owned receipt handles.

use std::mem::{align_of, size_of};
use std::time::Duration;

use mado_pilot::{
    CapabilitySupport, CleanupBudget, CleanupState, CoordinateSpace, DeliveryPlan,
    DiagnosticCategory, FocusPolicy, GeometryPolicy, InputCapability, InputDelivery,
    InputDescriptor, InputEvent, InputFault, InputOpenRequest, InputOperationKind, InputReceipt,
    InputRequest, InputRequirement, InputRouteCapability, InputSequence, Key, Modifier,
    PermissionKind, PermissionOutcome, PermissionState, Point, PointerButton, PointerGeometry,
    SequenceOutcome, SubmissionEvidence, TargetKind,
};

use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{FrameHandle, SessionHandle, madopilot_session_t};
use crate::engine::{
    EngineHandle, TargetList, madopilot_engine_t, madopilot_target_list_t, report,
};
use crate::error::{self, Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_INPUT, MADOPILOT_ERROR_CATEGORY_PERMISSION,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t,
};
use crate::types::{space_code, *};
use crate::view::{madopilot_bytes_t, madopilot_str_t};
use crate::{handle, hooks};

opaque! {
    /// One immutable owned input receipt.
    madopilot_input_receipt_t => InputReceiptHandle
}

#[derive(Debug)]
pub(crate) struct InputReceiptHandle {
    receipt: InputReceipt,
}

inputs! {
    impl Input for madopilot_input_open_request_t {
        const MANDATORY: usize = 32;
        const NAME: &'static str = "madopilot_input_open_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_input_open_request_t,
            struct_size,
            flags,
            requirement,
            reserved,
            required_pairs,
            preferred_pairs,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                requirement: MADOPILOT_INPUT_OPTIONAL,
                reserved: 0,
                required_pairs: 0,
                preferred_pairs: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }

    impl Input for madopilot_input_event_t {
        const MANDATORY: usize = 8;
        const NAME: &'static str = "madopilot_input_event_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_input_event_t,
            struct_size,
            kind,
            space,
            button,
            key,
            key_value,
            x,
            y,
            horizontal,
            vertical,
            text,
            delay_nanos,
        );
        const PRESENCE: &'static [(u32, usize)] = &[];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                kind: MADOPILOT_INPUT_EVENT_UNKNOWN,
                space: MADOPILOT_SPACE_CAPTURE_PIXELS,
                button: MADOPILOT_POINTER_BUTTON_UNKNOWN,
                key: MADOPILOT_KEY_UNKNOWN,
                key_value: 0,
                x: 0.0,
                y: 0.0,
                horizontal: 0,
                vertical: 0,
                text: madopilot_str_t::empty(),
                delay_nanos: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            0
        }
    }

    impl Input for madopilot_input_request_t {
        const MANDATORY: usize = 64;
        const NAME: &'static str = "madopilot_input_request_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_input_request_t,
            struct_size,
            flags,
            events,
            event_count,
            event_stride,
            deliveries,
            delivery_count,
            focus_policy,
            geometry_policy,
            source_frame,
            cleanup_max_events,
            reserved,
            cleanup_timeout_nanos,
        );
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET,
            covers!(madopilot_input_request_t, cleanup_timeout_nanos: u64),
        )];

        fn defaults() -> Self {
            Self {
                struct_size: 0,
                flags: 0,
                events: std::ptr::null(),
                event_count: 0,
                event_stride: 0,
                deliveries: std::ptr::null(),
                delivery_count: 0,
                focus_policy: MADOPILOT_FOCUS_PRESERVE,
                geometry_policy: MADOPILOT_GEOMETRY_REPROJECT_CURRENT,
                source_frame: std::ptr::null(),
                cleanup_max_events: 0,
                reserved: 0,
                cleanup_timeout_nanos: 0,
            }
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_engine_capabilities_t {
    const MANDATORY: usize = 8;
    const NAME: &'static str = "madopilot_engine_capabilities_t";
    const PREFIXES: &'static [usize] =
        prefixes!(madopilot_engine_capabilities_t, struct_size, flags);
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
        }
    }
}

impl Versioned for madopilot_permission_t {
    const MANDATORY: usize = 16;
    const NAME: &'static str = "madopilot_permission_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_permission_t,
        struct_size,
        flags,
        kind,
        state,
        diagnostic_category,
        reserved,
        platform_code,
        platform_namespace,
        context,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            kind: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            state: MADOPILOT_PERMISSION_STATE_UNKNOWN,
            diagnostic_category: MADOPILOT_DIAGNOSTIC_UNSPECIFIED,
            reserved: 0,
            platform_code: 0,
            platform_namespace: madopilot_str_t::empty(),
            context: madopilot_str_t::empty(),
        }
    }
}

impl Versioned for madopilot_input_capability_t {
    const MANDATORY: usize = 28;
    const NAME: &'static str = "madopilot_input_capability_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_capability_t,
        struct_size,
        flags,
        target,
        operation,
        delivery,
        support,
        address_scope,
        permission,
        evidence,
        focus_required,
        pointer_spaces,
        reserved,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[(
        covers!(madopilot_input_capability_t, reserved: u32),
        size_of::<madopilot_input_capability_t>(),
    )];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            operation: MADOPILOT_INPUT_OPERATION_UNKNOWN,
            delivery: MADOPILOT_INPUT_DELIVERY_NONE,
            support: MADOPILOT_CAPABILITY_UNKNOWN,
            address_scope: MADOPILOT_INPUT_ADDRESS_NONE,
            permission: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            evidence: MADOPILOT_SUBMISSION_EVIDENCE_NONE,
            focus_required: 0,
            pointer_spaces: 0,
            reserved: 0,
        }
    }
}

impl Versioned for madopilot_input_descriptor_t {
    const MANDATORY: usize = 48;
    const NAME: &'static str = "madopilot_input_descriptor_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_descriptor_t,
        struct_size,
        flags,
        target,
        known_pairs,
        supported_pairs,
        unknown_pairs,
        pointer_spaces,
        max_events,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            known_pairs: 0,
            supported_pairs: 0,
            unknown_pairs: 0,
            pointer_spaces: 0,
            max_events: 0,
        }
    }
}

impl Versioned for madopilot_input_receipt_info_t {
    const MANDATORY: usize = 88;
    const NAME: &'static str = "madopilot_input_receipt_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_receipt_info_t,
        struct_size,
        flags,
        target,
        outcome,
        selected_route,
        address_scope,
        attempt_count,
        submitted,
        last_submitted,
        evidence,
        fault,
        cleanup,
        cleanup_released,
        cleanup_owed,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[
        (
            covers!(
                madopilot_input_receipt_info_t,
                address_scope: madopilot_input_address_scope_t
            ),
            std::mem::offset_of!(madopilot_input_receipt_info_t, attempt_count),
        ),
        (
            covers!(
                madopilot_input_receipt_info_t,
                cleanup: madopilot_cleanup_state_t
            ),
            std::mem::offset_of!(madopilot_input_receipt_info_t, cleanup_released),
        ),
    ];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            outcome: MADOPILOT_SEQUENCE_UNEXECUTED,
            selected_route: MADOPILOT_INPUT_DELIVERY_NONE,
            address_scope: MADOPILOT_INPUT_ADDRESS_NONE,
            attempt_count: 0,
            submitted: 0,
            last_submitted: 0,
            evidence: MADOPILOT_SUBMISSION_EVIDENCE_NONE,
            fault: MADOPILOT_INPUT_FAULT_NONE,
            cleanup: MADOPILOT_CLEANUP_NOT_NEEDED,
            cleanup_released: 0,
            cleanup_owed: 0,
        }
    }
}

impl Versioned for madopilot_input_attempt_t {
    const MANDATORY: usize = 56;
    const NAME: &'static str = "madopilot_input_attempt_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_attempt_t,
        struct_size,
        flags,
        route,
        address_scope,
        outcome,
        submitted,
        last_submitted,
        evidence,
        fault,
        reserved,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[
        (
            covers!(
                madopilot_input_attempt_t,
                outcome: madopilot_sequence_outcome_t
            ),
            std::mem::offset_of!(madopilot_input_attempt_t, submitted),
        ),
        (
            covers!(madopilot_input_attempt_t, reserved: u32),
            size_of::<madopilot_input_attempt_t>(),
        ),
    ];

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            route: MADOPILOT_INPUT_DELIVERY_NONE,
            address_scope: MADOPILOT_INPUT_ADDRESS_NONE,
            outcome: MADOPILOT_SEQUENCE_UNEXECUTED,
            submitted: 0,
            last_submitted: 0,
            evidence: MADOPILOT_SUBMISSION_EVIDENCE_NONE,
            fault: MADOPILOT_INPUT_FAULT_NONE,
            reserved: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CapabilityFields {
    pub(crate) known_pairs: u64,
    pub(crate) supported_pairs: u64,
    pub(crate) unknown_pairs: u64,
    pub(crate) spaces: u32,
}

pub(crate) fn capability_fields(capability: InputCapability) -> CapabilityFields {
    let mut fields = CapabilityFields {
        known_pairs: 0,
        supported_pairs: 0,
        unknown_pairs: 0,
        spaces: 0,
    };
    for (bit, operation, route) in known_pairs() {
        let pair = capability.pair(operation, route);
        match pair.support() {
            CapabilitySupport::Supported => {
                fields.known_pairs |= bit;
                fields.supported_pairs |= bit;
            }
            CapabilitySupport::Unsupported => fields.known_pairs |= bit,
            CapabilitySupport::Unknown => fields.unknown_pairs |= bit,
            _ => {}
        }
        if operation == InputOperationKind::Pointer {
            fields.spaces |= pointer_spaces(pair);
        }
    }
    fields
}

pub(crate) fn descriptor(
    target: u64,
    descriptor: &InputDescriptor,
    struct_size: u32,
) -> Result<madopilot_input_descriptor_t, Fault> {
    let fields = capability_fields(descriptor.capability());
    let max_events = u32::try_from(descriptor.limits().max_events())
        .map_err(|_| Fault::internal("the facade input event limit exceeds uint32_t"))?;
    Ok(madopilot_input_descriptor_t {
        struct_size,
        flags: 0,
        target,
        known_pairs: fields.known_pairs,
        supported_pairs: fields.supported_pairs,
        unknown_pairs: fields.unknown_pairs,
        pointer_spaces: fields.spaces,
        max_events,
    })
}

pub(crate) unsafe fn open_request(
    request: *const madopilot_input_open_request_t,
) -> Result<InputOpenRequest, Fault> {
    // SAFETY: forwarded unchanged from this function's own contract.
    let request = unsafe { boundary::read_input(request) }?;
    if request.flags != 0 {
        return Err(Fault::abi(format!(
            "madopilot_input_open_request_t sets unknown flags {:#x}",
            request.flags
        )));
    }
    if request.reserved != 0 {
        return Err(Fault::abi(
            "madopilot_input_open_request_t.reserved must be zero",
        ));
    }
    if request.required_pairs & !MADOPILOT_INPUT_PAIRS_ALL != 0
        || request.preferred_pairs & !MADOPILOT_INPUT_PAIRS_ALL != 0
    {
        return Err(Fault::abi("an input-open pair mask contains unknown bits"));
    }
    let requirement = match request.requirement {
        MADOPILOT_INPUT_OPTIONAL => InputRequirement::Optional,
        MADOPILOT_INPUT_REQUIRED => InputRequirement::Required,
        other => {
            return Err(Fault::abi(format!(
                "unrecognized input requirement {other}"
            )));
        }
    };
    let mut converted = InputOpenRequest::new().with_requirement(requirement);
    for (bit, kind, route) in known_pairs() {
        if request.required_pairs & bit != 0 {
            converted = converted.requiring(kind, route);
        }
        if request.preferred_pairs & bit != 0 {
            converted = converted.preferring(kind, route);
        }
    }
    Ok(converted)
}

pub(crate) fn engine_capabilities(
    engine: *const madopilot_engine_t,
    out_capabilities: *mut madopilot_engine_capabilities_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_capabilities) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let mut flags = 0;
    if engine.delivers_input() {
        flags |= MADOPILOT_ENGINE_DELIVERS_INPUT;
    }
    if engine.reads_permissions() {
        flags |= MADOPILOT_ENGINE_READS_PERMISSIONS;
    }
    if engine.retained_ocr_backend().is_some() {
        flags |= MADOPILOT_ENGINE_HAS_OCR;
    }
    // SAFETY: `out` was validated above.
    unsafe {
        out.commit(madopilot_engine_capabilities_t {
            struct_size: out.declared_size(),
            flags,
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn engine_permission(
    engine: *const madopilot_engine_t,
    kind: madopilot_permission_kind_t,
    operation: *const madopilot_operation_t,
    out_permission: *mut madopilot_permission_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    let out = match unsafe { boundary::begin_record_outputs(out_permission, out_error) } {
        Ok(out) => out,
        Err(status) => return status,
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_engine_permission(engine, kind, operation, &out),
        )
    }
}

fn run_engine_permission(
    engine: *const madopilot_engine_t,
    kind: madopilot_permission_kind_t,
    operation: *const madopilot_operation_t,
    out: &Out<madopilot_permission_t>,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the engine retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    let kind = permission_kind(kind)?;
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let outcome = engine
        .permission(kind, context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_PERMISSION))?;
    context.commit()?;
    // SAFETY: `out` was validated before this helper was called.
    unsafe { out.commit(permission_record(outcome, out.declared_size())) };
    Ok(())
}

pub(crate) fn target_list_input_capability(
    targets: *const madopilot_target_list_t,
    index: usize,
    operation: madopilot_input_operation_kind_t,
    delivery: madopilot_input_delivery_t,
    out_capability: *mut madopilot_input_capability_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_capability) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the target list retained for the call.
    let Some(targets) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let index = match boundary::index_within(index, targets.targets().len(), "target") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let operation = match input_operation(operation) {
        Ok(value) => value,
        Err(fault) => return fault.status(),
    };
    let delivery = match input_delivery(delivery) {
        Ok(value) => value,
        Err(fault) => return fault.status(),
    };
    let record = &targets.targets()[index];
    let pair = record
        .description()
        .capability()
        .input()
        .pair(operation, delivery);
    let value = capability_record(record.ordinal(), pair, out.declared_size());
    // SAFETY: `out` was validated above.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn engine_input_descriptor(
    engine: *const madopilot_engine_t,
    targets: *const madopilot_target_list_t,
    index: usize,
    operation: *const madopilot_operation_t,
    out_descriptor: *mut madopilot_input_descriptor_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    let out = match unsafe { boundary::begin_record_outputs(out_descriptor, out_error) } {
        Ok(out) => out,
        Err(status) => return status,
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_engine_input_descriptor(engine, targets, index, operation, &out),
        )
    }
}

fn run_engine_input_descriptor(
    engine: *const madopilot_engine_t,
    targets: *const madopilot_target_list_t,
    index: usize,
    operation: *const madopilot_operation_t,
    out: &Out<madopilot_input_descriptor_t>,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps both handles retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: as above.
    let Some(targets) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return Err(Fault::abi("`targets` is null"));
    };
    let index = boundary::index_within(index, targets.targets().len(), "target")?;
    let target = &targets.targets()[index];
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let described = engine
        .describe_input(target.facade_id(), context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_INPUT))?;
    context.commit()?;
    let value = descriptor(target.ordinal(), &described, out.declared_size())?;
    // SAFETY: `out` was validated before this helper was called.
    unsafe { out.commit(value) };
    Ok(())
}

pub(crate) fn session_input_descriptor(
    session: *const madopilot_session_t,
    out_descriptor: *mut madopilot_input_descriptor_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_descriptor) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the session retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let mut value = *session.input_descriptor();
    value.struct_size = out.declared_size();
    // SAFETY: `out` was validated above.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn session_send_input(
    session: *const madopilot_session_t,
    request: *const madopilot_input_request_t,
    operation: *const madopilot_operation_t,
    out_receipt: *mut *mut madopilot_input_receipt_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable, correctly aligned output addresses.
    if let Err(status) = unsafe { boundary::begin_outputs(out_receipt, "out_receipt", out_error) } {
        return status;
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: `out_error` was validated above.
    unsafe {
        report(
            out_error,
            run_session_send_input(session, request, operation, out_receipt),
        )
    }
}

fn run_session_send_input(
    session: *const madopilot_session_t,
    request: *const madopilot_input_request_t,
    operation: *const madopilot_operation_t,
    out_receipt: *mut *mut madopilot_input_receipt_t,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the session retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the caller keeps the request and its selected pointer views
    // readable, and every handle it names retained, for the call.
    let request = unsafe { input_request(request, session) }?;
    // SAFETY: the caller keeps the operation structure readable for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let receipt = session
        .session()
        .send_input(&request, context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_INPUT))?;
    hooks::reach(hooks::Site::AfterTemporary);
    let receipt = InputReceiptHandle { receipt };
    // SAFETY: `out_receipt` was validated before this helper was called.
    unsafe { out_receipt.write(handle::into_raw(receipt)) };
    Ok(())
}

pub(crate) fn receipt_retain(receipt: *const madopilot_input_receipt_t) -> madopilot_status_t {
    // SAFETY: the handle is null or one this module produced, and the caller
    // holds a live reference for the call.
    unsafe { handle::retain::<InputReceiptHandle>(receipt) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn receipt_release(receipt: *mut madopilot_input_receipt_t) -> madopilot_status_t {
    // SAFETY: as `receipt_retain`, and the caller is giving up its reference.
    unsafe { handle::release::<InputReceiptHandle>(receipt) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn receipt_info(
    receipt: *const madopilot_input_receipt_t,
    out_info: *mut madopilot_input_receipt_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the receipt retained for the call.
    let Some(receipt) = (unsafe { handle::borrow::<InputReceiptHandle>(receipt) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let value = match receipt_record(receipt, out.declared_size()) {
        Ok(value) => value,
        Err(fault) => return fault.status(),
    };
    // SAFETY: `out` was validated above.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn receipt_attempt_count(
    receipt: *const madopilot_input_receipt_t,
    out_count: *mut usize,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable, correctly aligned output address.
    if let Err(fault) = unsafe { boundary::begin_scalar_out(out_count, "out_count", 0_usize) } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the receipt retained for the call.
    let Some(receipt) = (unsafe { handle::borrow::<InputReceiptHandle>(receipt) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    // SAFETY: `out_count` was validated above.
    unsafe { boundary::commit_scalar(out_count, receipt.receipt.attempts().len()) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn receipt_attempt_at(
    receipt: *const madopilot_input_receipt_t,
    index: usize,
    out_attempt: *mut madopilot_input_attempt_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies a writable output structure whose
    // `struct_size` it has set.
    let out = match unsafe { Out::begin(out_attempt) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps the receipt retained for the call.
    let Some(receipt) = (unsafe { handle::borrow::<InputReceiptHandle>(receipt) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let index = match boundary::index_within(index, receipt.receipt.attempts().len(), "attempt") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let value = match attempt_record(receipt.receipt.attempts()[index], out.declared_size()) {
        Ok(value) => value,
        Err(fault) => return fault.status(),
    };
    // SAFETY: `out` was validated above.
    unsafe { out.commit(value) };
    MADOPILOT_STATUS_OK
}

fn permission_record(outcome: PermissionOutcome, struct_size: u32) -> madopilot_permission_t {
    let mut value = madopilot_permission_t::failure(struct_size);
    value.kind = permission_kind_code(outcome.kind());
    value.state = permission_state_code(outcome.state());
    let diagnostic_extent = covers!(madopilot_permission_t, context: madopilot_str_t);
    if struct_size as usize >= diagnostic_extent
        && let Some(diagnostic) = outcome.diagnostic()
    {
        value.flags |= MADOPILOT_PERMISSION_HAS_DIAGNOSTIC;
        value.diagnostic_category = diagnostic_category_code(diagnostic.category());
        value.context = madopilot_str_t::borrowed(diagnostic.context());
        if let Some(platform) = diagnostic.platform() {
            value.flags |= MADOPILOT_PERMISSION_HAS_PLATFORM_CODE;
            value.platform_code = platform.code();
            value.platform_namespace = madopilot_str_t::borrowed(platform.namespace());
        }
    }
    value
}

fn capability_record(
    target: u64,
    pair: InputRouteCapability,
    struct_size: u32,
) -> madopilot_input_capability_t {
    let mut flags = 0;
    let permission = pair
        .permission()
        .map_or(MADOPILOT_PERMISSION_KIND_UNSPECIFIED, |value| {
            flags |= MADOPILOT_INPUT_CAPABILITY_HAS_PERMISSION;
            permission_kind_code(value)
        });
    let evidence = pair
        .evidence()
        .map_or(MADOPILOT_SUBMISSION_EVIDENCE_NONE, |value| {
            flags |= MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE;
            submission_evidence_code(value)
        });
    madopilot_input_capability_t {
        struct_size,
        flags,
        target,
        operation: input_operation_code(pair.operation()),
        delivery: input_delivery_code(pair.route()),
        support: capability_support_code(pair.support()),
        address_scope: input_address_scope_code(pair.address_scope()),
        permission,
        evidence,
        focus_required: i32::from(pair.focus_required()),
        pointer_spaces: pointer_spaces(pair),
        reserved: 0,
    }
}

unsafe fn input_request(
    request: *const madopilot_input_request_t,
    session: &SessionHandle,
) -> Result<InputRequest, Fault> {
    // SAFETY: forwarded unchanged from this function's own contract.
    let request = unsafe { boundary::read_input(request) }?;
    if request.flags & !MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET != 0 {
        return Err(Fault::abi(format!(
            "madopilot_input_request_t sets unknown flags {:#x}",
            request.flags
        )));
    }
    if request.reserved != 0 {
        return Err(Fault::abi(
            "madopilot_input_request_t.reserved must be zero",
        ));
    }
    let limits = session.session().input_descriptor().limits();
    // SAFETY: the caller keeps the event array and every selected nested view
    // readable for the call.
    let events = unsafe {
        input_events(
            request.events,
            request.event_count,
            request.event_stride,
            limits.max_events(),
        )
    }?;
    let has_pointer = events.iter().any(|event| {
        matches!(
            event,
            InputEvent::PointerMove(_)
                | InputEvent::PointerPress(_)
                | InputEvent::PointerRelease(_)
                | InputEvent::PointerScroll { .. }
        )
    });
    let sequence = InputSequence::within(events, limits).map_err(input_fault)?;
    // SAFETY: the caller keeps the delivery array readable for the call.
    let deliveries = unsafe { delivery_plan(request.deliveries, request.delivery_count) }?;
    let focus = focus_policy(request.focus_policy)?;
    let geometry_policy = geometry_policy(request.geometry_policy)?;
    let geometry = if has_pointer {
        // SAFETY: the caller keeps a non-null source-frame handle retained for
        // geometry policies that require one.
        unsafe { pointer_geometry(geometry_policy, request.source_frame) }?
    } else {
        PointerGeometry::reprojected()
    };
    let cleanup = if declared!(
        request,
        madopilot_input_request_t,
        MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET
    ) {
        if request.cleanup_max_events > MADOPILOT_INPUT_MAX_CLEANUP_EVENTS
            || request.cleanup_timeout_nanos > MADOPILOT_INPUT_MAX_CLEANUP_NANOS
        {
            return Err(input_fault(InputFault::SequenceOutOfBounds));
        }
        CleanupBudget::at_most(
            request.cleanup_max_events as usize,
            Duration::from_nanos(request.cleanup_timeout_nanos),
        )
    } else {
        CleanupBudget::contract()
    };
    let request = InputRequest::new(session.target(), sequence, deliveries)
        .with_focus(focus)
        .with_pointer_geometry(geometry)
        .with_cleanup_budget(cleanup);
    request.check().map_err(input_fault)?;
    Ok(request)
}

unsafe fn input_events(
    events: *const madopilot_input_event_t,
    count: usize,
    stride: usize,
    max_events: usize,
) -> Result<Vec<InputEvent>, Fault> {
    if count > max_events {
        return Err(input_fault(InputFault::SequenceOutOfBounds));
    }
    boundary::span(
        count,
        stride,
        <madopilot_input_event_t as boundary::Input>::MANDATORY,
        "events",
    )?;
    if count > 0 {
        if events.is_null() {
            return Err(Fault::abi("`events` is null for a nonzero event count"));
        }
        if !events
            .addr()
            .is_multiple_of(align_of::<madopilot_input_event_t>())
        {
            return Err(Fault::abi(format!(
                "`events` is not aligned to {} bytes",
                align_of::<madopilot_input_event_t>()
            )));
        }
    }
    let mut converted = Vec::with_capacity(count);
    for index in 0..count {
        let event =
        // SAFETY: `span` proved that `index * stride` stays inside one
        // representable object, and the caller contract keeps it readable.
            unsafe { events.cast::<u8>().add(index * stride) }.cast::<madopilot_input_event_t>();
        // SAFETY: as above. `read_element` validates the declared size and reads
        // no farther than this element's stride.
        let event = unsafe { boundary::read_element(event, stride) }?;
        // SAFETY: the caller keeps any nested view selected by the event tag
        // readable for the call.
        converted.push(unsafe { input_event(event, index) }?);
    }
    Ok(converted)
}

unsafe fn input_event(event: madopilot_input_event_t, index: usize) -> Result<InputEvent, Fault> {
    match event.kind {
        MADOPILOT_INPUT_EVENT_POINTER_MOVE => {
            require_event_prefix(&event, covers!(madopilot_input_event_t, y: f64), index)?;
            let space = coordinate_space(event.space)?;
            let point = Point::new(space, event.x, event.y).map_err(|fault| {
                Fault::abi(format!("event {index} has an invalid point: {fault}"))
            })?;
            Ok(InputEvent::PointerMove(point))
        }
        MADOPILOT_INPUT_EVENT_POINTER_PRESS | MADOPILOT_INPUT_EVENT_POINTER_RELEASE => {
            require_event_prefix(
                &event,
                covers!(madopilot_input_event_t, button: madopilot_pointer_button_t),
                index,
            )?;
            let button = pointer_button(event.button)?;
            if event.kind == MADOPILOT_INPUT_EVENT_POINTER_PRESS {
                Ok(InputEvent::PointerPress(button))
            } else {
                Ok(InputEvent::PointerRelease(button))
            }
        }
        MADOPILOT_INPUT_EVENT_POINTER_SCROLL => {
            require_event_prefix(
                &event,
                covers!(madopilot_input_event_t, vertical: i32),
                index,
            )?;
            let horizontal = i16::try_from(event.horizontal)
                .map_err(|_| input_fault(InputFault::SequenceOutOfBounds))?;
            let vertical = i16::try_from(event.vertical)
                .map_err(|_| input_fault(InputFault::SequenceOutOfBounds))?;
            Ok(InputEvent::PointerScroll {
                horizontal,
                vertical,
            })
        }
        MADOPILOT_INPUT_EVENT_KEY_PRESS | MADOPILOT_INPUT_EVENT_KEY_RELEASE => {
            require_event_prefix(
                &event,
                covers!(madopilot_input_event_t, key_value: u32),
                index,
            )?;
            let key = key(event.key, event.key_value)?;
            if event.kind == MADOPILOT_INPUT_EVENT_KEY_PRESS {
                Ok(InputEvent::KeyPress(key))
            } else {
                Ok(InputEvent::KeyRelease(key))
            }
        }
        MADOPILOT_INPUT_EVENT_TEXT => {
            require_event_prefix(
                &event,
                covers!(madopilot_input_event_t, text: madopilot_str_t),
                index,
            )?;
            let raw = madopilot_bytes_t {
                data: event.text.data.cast(),
                len: event.text.len,
            };
            let byte_len = crate::view::byte_len(raw, "event.text")?;
            if byte_len > MADOPILOT_INPUT_MAX_TEXT_UTF8_BYTES as usize {
                return Err(input_fault(InputFault::SequenceOutOfBounds));
            }
            // SAFETY: forwarded unchanged from this function's own contract
            // after validating the selected text view's bounded byte length.
            let text = unsafe { crate::view::string(event.text, "event.text") }?;
            if text.chars().count() > MADOPILOT_INPUT_MAX_TEXT_CHARS as usize {
                return Err(input_fault(InputFault::SequenceOutOfBounds));
            }
            Ok(InputEvent::Text(text.to_owned()))
        }
        MADOPILOT_INPUT_EVENT_DELAY => {
            require_event_prefix(
                &event,
                covers!(madopilot_input_event_t, delay_nanos: u64),
                index,
            )?;
            Ok(InputEvent::Delay(Duration::from_nanos(event.delay_nanos)))
        }
        other => Err(Fault::abi(format!(
            "event {index} has unrecognized input kind {other}"
        ))),
    }
}

fn require_event_prefix(
    event: &madopilot_input_event_t,
    required: usize,
    index: usize,
) -> Result<(), Fault> {
    if event.struct_size as usize >= required {
        Ok(())
    } else {
        Err(Fault::abi(format!(
            "event {index} declares {} bytes, below its {required} byte variant prefix",
            event.struct_size
        )))
    }
}

unsafe fn delivery_plan(
    deliveries: *const madopilot_input_delivery_t,
    count: usize,
) -> Result<DeliveryPlan, Fault> {
    if count > InputDelivery::ALL.len() {
        return Err(input_fault(InputFault::InvalidRoutePlan));
    }
    boundary::span(
        count,
        size_of::<madopilot_input_delivery_t>(),
        size_of::<madopilot_input_delivery_t>(),
        "deliveries",
    )?;
    if count > 0 {
        if deliveries.is_null() {
            return Err(Fault::abi(
                "`deliveries` is null for a nonzero delivery count",
            ));
        }
        if !deliveries
            .addr()
            .is_multiple_of(align_of::<madopilot_input_delivery_t>())
        {
            return Err(Fault::abi(format!(
                "`deliveries` is not aligned to {} bytes",
                align_of::<madopilot_input_delivery_t>()
            )));
        }
    }
    let mut converted = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: `span` proved the indexed element lies in the caller's
        // readable, correctly aligned delivery array.
        converted.push(input_delivery(unsafe { deliveries.add(index).read() })?);
    }
    DeliveryPlan::ordered(converted).map_err(input_fault)
}

unsafe fn pointer_geometry(
    policy: GeometryPolicy,
    source_frame: *const crate::capture::madopilot_frame_t,
) -> Result<PointerGeometry, Fault> {
    match policy {
        GeometryPolicy::ReprojectCurrent => Ok(PointerGeometry::reprojected()),
        GeometryPolicy::RequireUnchanged | GeometryPolicy::UseFrameSnapshot => {
            // SAFETY: the caller keeps the source-frame handle retained for the
            // call when this policy requires one.
            let Some(frame) = (unsafe { handle::borrow::<FrameHandle>(source_frame) }) else {
                return Err(input_fault(InputFault::MissingCoordinateSource));
            };
            let stamp = frame.frame().stamp();
            if matches!(policy, GeometryPolicy::RequireUnchanged) {
                Ok(PointerGeometry::require_unchanged_since(stamp))
            } else {
                Ok(PointerGeometry::from_frame_snapshot(stamp))
            }
        }
        _ => Err(Fault::abi("unrecognized pointer geometry policy")),
    }
}

fn receipt_record(
    handle: &InputReceiptHandle,
    struct_size: u32,
) -> Result<madopilot_input_receipt_info_t, Fault> {
    let receipt = &handle.receipt;
    let mut flags = 0;
    let selected_route = receipt
        .selected_route()
        .map_or(MADOPILOT_INPUT_DELIVERY_NONE, |route| {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE;
            input_delivery_code(route)
        });
    let address_scope = receipt
        .address_scope()
        .map_or(MADOPILOT_INPUT_ADDRESS_NONE, |scope| {
            input_address_scope_code(scope)
        });
    let last_submitted = match receipt.last_submitted() {
        Some(index) => {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_LAST_SUBMITTED;
            semantic_count(index, "last submitted input index")?
        }
        None => 0,
    };
    let evidence = receipt
        .evidence()
        .map_or(MADOPILOT_SUBMISSION_EVIDENCE_NONE, |evidence| {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE;
            submission_evidence_code(evidence)
        });
    let fault = receipt.fault().map_or(MADOPILOT_INPUT_FAULT_NONE, |fault| {
        flags |= MADOPILOT_INPUT_RECEIPT_HAS_FAULT;
        input_fault_code(fault)
    });
    if receipt.partial_native_effect() {
        flags |= MADOPILOT_INPUT_RECEIPT_PARTIAL_NATIVE_EFFECT;
    }
    if receipt.used_fallback() {
        flags |= MADOPILOT_INPUT_RECEIPT_USED_FALLBACK;
    }
    Ok(madopilot_input_receipt_info_t {
        struct_size,
        flags,
        target: receipt.target().get(),
        outcome: sequence_outcome_code(receipt.outcome()),
        selected_route,
        address_scope,
        attempt_count: semantic_count(receipt.attempts().len(), "input attempt count")?,
        submitted: semantic_count(receipt.submitted(), "submitted input count")?,
        last_submitted,
        evidence,
        fault,
        cleanup: cleanup_state_code(receipt.cleanup()),
        cleanup_released: semantic_count(receipt.cleanup_released(), "cleanup release count")?,
        cleanup_owed: semantic_count(receipt.cleanup_owed(), "cleanup owed count")?,
    })
}

fn attempt_record(
    attempt: mado_pilot::InputAttempt,
    struct_size: u32,
) -> Result<madopilot_input_attempt_t, Fault> {
    let mut flags = 0;
    let last_submitted = match attempt.last_submitted() {
        Some(index) => {
            flags |= MADOPILOT_INPUT_ATTEMPT_HAS_LAST_SUBMITTED;
            semantic_count(index, "last submitted input index")?
        }
        None => 0,
    };
    let evidence = attempt
        .evidence()
        .map_or(MADOPILOT_SUBMISSION_EVIDENCE_NONE, |evidence| {
            flags |= MADOPILOT_INPUT_ATTEMPT_HAS_EVIDENCE;
            submission_evidence_code(evidence)
        });
    let fault = attempt.fault().map_or(MADOPILOT_INPUT_FAULT_NONE, |fault| {
        flags |= MADOPILOT_INPUT_ATTEMPT_HAS_FAULT;
        input_fault_code(fault)
    });
    if attempt.partial_native_effect() {
        flags |= MADOPILOT_INPUT_ATTEMPT_PARTIAL_NATIVE_EFFECT;
    }
    Ok(madopilot_input_attempt_t {
        struct_size,
        flags,
        route: input_delivery_code(attempt.route()),
        address_scope: input_address_scope_code(attempt.address_scope()),
        outcome: sequence_outcome_code(attempt.outcome()),
        submitted: semantic_count(attempt.submitted(), "attempt submitted count")?,
        last_submitted,
        evidence,
        fault,
        reserved: 0,
    })
}

fn input_fault(fault: InputFault) -> Fault {
    let error: mado_pilot::Error = fault.into();
    Fault::from_error(&error, MADOPILOT_ERROR_CATEGORY_INPUT)
}

fn semantic_count(value: usize, field: &'static str) -> Result<u64, Fault> {
    u64::try_from(value)
        .map_err(|_| Fault::internal(format!("the facade {field} exceeds uint64_t")))
}

fn permission_kind(value: madopilot_permission_kind_t) -> Result<PermissionKind, Fault> {
    match value {
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE => Ok(PermissionKind::ScreenCapture),
        MADOPILOT_PERMISSION_KIND_INPUT_CONTROL => Ok(PermissionKind::InputControl),
        other => Err(Fault::abi(format!("unrecognized permission kind {other}"))),
    }
}

pub(crate) const fn permission_state_code(state: PermissionState) -> madopilot_permission_state_t {
    match state {
        PermissionState::Granted => MADOPILOT_PERMISSION_STATE_GRANTED,
        PermissionState::NotGranted => MADOPILOT_PERMISSION_STATE_NOT_GRANTED,
        PermissionState::Unavailable => MADOPILOT_PERMISSION_STATE_UNAVAILABLE,
        PermissionState::Unknown => MADOPILOT_PERMISSION_STATE_UNKNOWN,
        _ => MADOPILOT_PERMISSION_STATE_UNKNOWN,
    }
}

const fn diagnostic_category_code(category: DiagnosticCategory) -> madopilot_diagnostic_category_t {
    match category {
        DiagnosticCategory::PermissionDenied => MADOPILOT_DIAGNOSTIC_PERMISSION_DENIED,
        DiagnosticCategory::PermissionUndetermined => MADOPILOT_DIAGNOSTIC_PERMISSION_UNDETERMINED,
        DiagnosticCategory::CapabilityUnavailable => MADOPILOT_DIAGNOSTIC_CAPABILITY_UNAVAILABLE,
        DiagnosticCategory::TargetLost => MADOPILOT_DIAGNOSTIC_TARGET_LOST,
        DiagnosticCategory::PlatformFailure => MADOPILOT_DIAGNOSTIC_PLATFORM_FAILURE,
        DiagnosticCategory::Configuration => MADOPILOT_DIAGNOSTIC_CONFIGURATION,
        _ => MADOPILOT_DIAGNOSTIC_UNSPECIFIED,
    }
}

fn coordinate_space(value: madopilot_space_t) -> Result<CoordinateSpace, Fault> {
    match value {
        MADOPILOT_SPACE_CAPTURE_PIXELS => Ok(CoordinateSpace::CapturePixels),
        MADOPILOT_SPACE_FRAME_NORMALIZED => Ok(CoordinateSpace::FrameNormalized),
        MADOPILOT_SPACE_TARGET_NORMALIZED => Ok(CoordinateSpace::TargetNormalized),
        MADOPILOT_SPACE_TARGET_LOGICAL => Ok(CoordinateSpace::TargetLogical),
        MADOPILOT_SPACE_DESKTOP_LOGICAL => Ok(CoordinateSpace::DesktopLogical),
        other => Err(Fault::abi(format!("unrecognized coordinate space {other}"))),
    }
}

fn pointer_button(value: madopilot_pointer_button_t) -> Result<PointerButton, Fault> {
    match value {
        MADOPILOT_POINTER_BUTTON_PRIMARY => Ok(PointerButton::Primary),
        MADOPILOT_POINTER_BUTTON_SECONDARY => Ok(PointerButton::Secondary),
        MADOPILOT_POINTER_BUTTON_MIDDLE => Ok(PointerButton::Middle),
        other => Err(Fault::abi(format!("unrecognized pointer button {other}"))),
    }
}

fn key(value: madopilot_key_t, key_value: u32) -> Result<Key, Fault> {
    match value {
        MADOPILOT_KEY_CHARACTER => char::from_u32(key_value)
            .map(Key::Character)
            .ok_or_else(|| input_fault(InputFault::SequenceOutOfBounds)),
        MADOPILOT_KEY_FUNCTION => u8::try_from(key_value)
            .map(Key::Function)
            .map_err(|_| input_fault(InputFault::SequenceOutOfBounds)),
        MADOPILOT_KEY_MODIFIER => Ok(Key::Modifier(modifier(key_value)?)),
        MADOPILOT_KEY_ENTER => Ok(Key::Enter),
        MADOPILOT_KEY_TAB => Ok(Key::Tab),
        MADOPILOT_KEY_BACKSPACE => Ok(Key::Backspace),
        MADOPILOT_KEY_DELETE => Ok(Key::Delete),
        MADOPILOT_KEY_ESCAPE => Ok(Key::Escape),
        MADOPILOT_KEY_SPACE => Ok(Key::Space),
        MADOPILOT_KEY_ARROW_UP => Ok(Key::ArrowUp),
        MADOPILOT_KEY_ARROW_DOWN => Ok(Key::ArrowDown),
        MADOPILOT_KEY_ARROW_LEFT => Ok(Key::ArrowLeft),
        MADOPILOT_KEY_ARROW_RIGHT => Ok(Key::ArrowRight),
        MADOPILOT_KEY_HOME => Ok(Key::Home),
        MADOPILOT_KEY_END => Ok(Key::End),
        MADOPILOT_KEY_PAGE_UP => Ok(Key::PageUp),
        MADOPILOT_KEY_PAGE_DOWN => Ok(Key::PageDown),
        other => Err(Fault::abi(format!("unrecognized key kind {other}"))),
    }
}

fn modifier(value: u32) -> Result<Modifier, Fault> {
    match value {
        x if x == MADOPILOT_MODIFIER_SHIFT as u32 => Ok(Modifier::Shift),
        x if x == MADOPILOT_MODIFIER_CONTROL as u32 => Ok(Modifier::Control),
        x if x == MADOPILOT_MODIFIER_ALT as u32 => Ok(Modifier::Alt),
        x if x == MADOPILOT_MODIFIER_META as u32 => Ok(Modifier::Meta),
        other => Err(Fault::abi(format!(
            "unrecognized keyboard modifier {other}"
        ))),
    }
}

pub(crate) fn input_operation(
    value: madopilot_input_operation_kind_t,
) -> Result<InputOperationKind, Fault> {
    match value {
        MADOPILOT_INPUT_OPERATION_POINTER => Ok(InputOperationKind::Pointer),
        MADOPILOT_INPUT_OPERATION_KEYBOARD => Ok(InputOperationKind::Keyboard),
        MADOPILOT_INPUT_OPERATION_TEXT => Ok(InputOperationKind::Text),
        other => Err(Fault::abi(format!("unrecognized input operation {other}"))),
    }
}

pub(crate) const fn input_operation_code(
    value: InputOperationKind,
) -> madopilot_input_operation_kind_t {
    match value {
        InputOperationKind::Pointer => MADOPILOT_INPUT_OPERATION_POINTER,
        InputOperationKind::Keyboard => MADOPILOT_INPUT_OPERATION_KEYBOARD,
        InputOperationKind::Text => MADOPILOT_INPUT_OPERATION_TEXT,
        _ => MADOPILOT_INPUT_OPERATION_UNKNOWN,
    }
}

fn input_delivery(value: madopilot_input_delivery_t) -> Result<InputDelivery, Fault> {
    match value {
        MADOPILOT_INPUT_DELIVERY_SYSTEM => Ok(InputDelivery::System),
        MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE => Ok(InputDelivery::WindowMessage),
        MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED => Ok(InputDelivery::ProcessDirected),
        other => Err(Fault::abi(format!("unrecognized input delivery {other}"))),
    }
}

pub(crate) const fn input_delivery_code(value: InputDelivery) -> madopilot_input_delivery_t {
    match value {
        InputDelivery::System => MADOPILOT_INPUT_DELIVERY_SYSTEM,
        InputDelivery::WindowMessage => MADOPILOT_INPUT_DELIVERY_WINDOW_MESSAGE,
        InputDelivery::ProcessDirected => MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED,
        _ => MADOPILOT_INPUT_DELIVERY_NONE,
    }
}

pub(crate) const fn input_address_scope_code(
    value: mado_pilot::InputAddressScope,
) -> madopilot_input_address_scope_t {
    match value {
        mado_pilot::InputAddressScope::FocusedSystem => MADOPILOT_INPUT_ADDRESS_FOCUSED_SYSTEM,
        mado_pilot::InputAddressScope::ExactWindow => MADOPILOT_INPUT_ADDRESS_EXACT_WINDOW,
        mado_pilot::InputAddressScope::OwningProcess => MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS,
        _ => MADOPILOT_INPUT_ADDRESS_NONE,
    }
}

pub(crate) const fn submission_evidence_code(
    value: SubmissionEvidence,
) -> madopilot_submission_evidence_t {
    match value {
        SubmissionEvidence::InvocationOnly => MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY,
        SubmissionEvidence::SystemInputAdmission => {
            MADOPILOT_SUBMISSION_EVIDENCE_SYSTEM_INPUT_ADMISSION
        }
        SubmissionEvidence::TargetQueueAdmission => {
            MADOPILOT_SUBMISSION_EVIDENCE_TARGET_QUEUE_ADMISSION
        }
        SubmissionEvidence::TargetProtocolAcknowledgement => {
            MADOPILOT_SUBMISSION_EVIDENCE_TARGET_PROTOCOL_ACKNOWLEDGEMENT
        }
        _ => MADOPILOT_SUBMISSION_EVIDENCE_NONE,
    }
}

fn focus_policy(value: madopilot_focus_policy_t) -> Result<FocusPolicy, Fault> {
    match value {
        MADOPILOT_FOCUS_PRESERVE => Ok(FocusPolicy::Preserve),
        MADOPILOT_FOCUS_REQUIRE_FOCUSED => Ok(FocusPolicy::RequireFocused),
        MADOPILOT_FOCUS_ACTIVATE_IF_REQUIRED => Ok(FocusPolicy::ActivateIfRequired),
        other => Err(Fault::abi(format!("unrecognized focus policy {other}"))),
    }
}

fn geometry_policy(value: madopilot_geometry_policy_t) -> Result<GeometryPolicy, Fault> {
    match value {
        MADOPILOT_GEOMETRY_REPROJECT_CURRENT => Ok(GeometryPolicy::ReprojectCurrent),
        MADOPILOT_GEOMETRY_REQUIRE_UNCHANGED => Ok(GeometryPolicy::RequireUnchanged),
        MADOPILOT_GEOMETRY_USE_FRAME_SNAPSHOT => Ok(GeometryPolicy::UseFrameSnapshot),
        other => Err(Fault::abi(format!("unrecognized geometry policy {other}"))),
    }
}

pub(crate) const fn sequence_outcome_code(value: SequenceOutcome) -> madopilot_sequence_outcome_t {
    match value {
        SequenceOutcome::Complete => MADOPILOT_SEQUENCE_COMPLETE,
        SequenceOutcome::Partial => MADOPILOT_SEQUENCE_PARTIAL,
        SequenceOutcome::Unexecuted => MADOPILOT_SEQUENCE_UNEXECUTED,
        _ => MADOPILOT_SEQUENCE_UNEXECUTED,
    }
}

pub(crate) const fn cleanup_state_code(value: CleanupState) -> madopilot_cleanup_state_t {
    match value {
        CleanupState::NotNeeded => MADOPILOT_CLEANUP_NOT_NEEDED,
        CleanupState::Complete => MADOPILOT_CLEANUP_COMPLETE,
        CleanupState::Incomplete => MADOPILOT_CLEANUP_INCOMPLETE,
        CleanupState::Exhausted => MADOPILOT_CLEANUP_EXHAUSTED,
        _ => MADOPILOT_CLEANUP_INCOMPLETE,
    }
}

pub(crate) const fn input_fault_code(value: InputFault) -> madopilot_input_fault_t {
    match value {
        InputFault::ForeignTarget => MADOPILOT_INPUT_FAULT_FOREIGN_TARGET,
        InputFault::UnknownTarget => MADOPILOT_INPUT_FAULT_UNKNOWN_TARGET,
        InputFault::TargetLost => MADOPILOT_INPUT_FAULT_TARGET_LOST,
        InputFault::ProviderMismatch => MADOPILOT_INPUT_FAULT_PROVIDER_MISMATCH,
        InputFault::UnsupportedCombination => MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION,
        InputFault::InvalidRoutePlan => MADOPILOT_INPUT_FAULT_INVALID_ROUTE_PLAN,
        InputFault::RouteUnavailable => MADOPILOT_INPUT_FAULT_ROUTE_UNAVAILABLE,
        InputFault::SequenceOutOfBounds => MADOPILOT_INPUT_FAULT_SEQUENCE_OUT_OF_BOUNDS,
        InputFault::UnsupportedCoordinate => MADOPILOT_INPUT_FAULT_UNSUPPORTED_COORDINATE,
        InputFault::MissingCoordinateSource => MADOPILOT_INPUT_FAULT_MISSING_COORDINATE_SOURCE,
        InputFault::GeometryChanged => MADOPILOT_INPUT_FAULT_GEOMETRY_CHANGED,
        InputFault::FocusRequired => MADOPILOT_INPUT_FAULT_FOCUS_REQUIRED,
        InputFault::FocusRefused => MADOPILOT_INPUT_FAULT_FOCUS_REFUSED,
        InputFault::NotAuthorized => MADOPILOT_INPUT_FAULT_NOT_AUTHORIZED,
        InputFault::PolicyRefused => MADOPILOT_INPUT_FAULT_POLICY_REFUSED,
        InputFault::ControllerClosed => MADOPILOT_INPUT_FAULT_CONTROLLER_CLOSED,
        InputFault::Cancelled => MADOPILOT_INPUT_FAULT_CANCELLED,
        InputFault::DeadlineExceeded => MADOPILOT_INPUT_FAULT_DEADLINE_EXCEEDED,
        InputFault::SubmissionFailed => MADOPILOT_INPUT_FAULT_SUBMISSION_FAILED,
        _ => MADOPILOT_INPUT_FAULT_SUBMISSION_FAILED,
    }
}

pub(crate) const fn target_kind_code(kind: TargetKind) -> madopilot_target_kind_t {
    match kind {
        TargetKind::Window => MADOPILOT_TARGET_KIND_WINDOW,
        TargetKind::Display => MADOPILOT_TARGET_KIND_DISPLAY,
        _ => MADOPILOT_TARGET_KIND_UNKNOWN,
    }
}

pub(crate) const fn capability_support_code(
    support: CapabilitySupport,
) -> madopilot_capability_support_t {
    match support {
        CapabilitySupport::Supported => MADOPILOT_CAPABILITY_SUPPORTED,
        CapabilitySupport::Unsupported => MADOPILOT_CAPABILITY_UNSUPPORTED,
        CapabilitySupport::Unknown => MADOPILOT_CAPABILITY_UNKNOWN,
        _ => MADOPILOT_CAPABILITY_UNKNOWN,
    }
}

pub(crate) const fn permission_kind_code(kind: PermissionKind) -> madopilot_permission_kind_t {
    match kind {
        PermissionKind::ScreenCapture => MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE,
        PermissionKind::InputControl => MADOPILOT_PERMISSION_KIND_INPUT_CONTROL,
        _ => MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
    }
}

const fn pair_bit(kind: InputOperationKind, route: InputDelivery) -> u64 {
    match (kind, route) {
        (InputOperationKind::Pointer, InputDelivery::System) => MADOPILOT_INPUT_PAIR_POINTER_SYSTEM,
        (InputOperationKind::Pointer, InputDelivery::WindowMessage) => {
            MADOPILOT_INPUT_PAIR_POINTER_WINDOW_MESSAGE
        }
        (InputOperationKind::Pointer, InputDelivery::ProcessDirected) => {
            MADOPILOT_INPUT_PAIR_POINTER_PROCESS_DIRECTED
        }
        (InputOperationKind::Keyboard, InputDelivery::System) => {
            MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM
        }
        (InputOperationKind::Keyboard, InputDelivery::WindowMessage) => {
            MADOPILOT_INPUT_PAIR_KEYBOARD_WINDOW_MESSAGE
        }
        (InputOperationKind::Keyboard, InputDelivery::ProcessDirected) => {
            MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED
        }
        (InputOperationKind::Text, InputDelivery::System) => MADOPILOT_INPUT_PAIR_TEXT_SYSTEM,
        (InputOperationKind::Text, InputDelivery::WindowMessage) => {
            MADOPILOT_INPUT_PAIR_TEXT_WINDOW_MESSAGE
        }
        (InputOperationKind::Text, InputDelivery::ProcessDirected) => {
            MADOPILOT_INPUT_PAIR_TEXT_PROCESS_DIRECTED
        }
        _ => 0,
    }
}

const fn known_pairs() -> [(u64, InputOperationKind, InputDelivery); 9] {
    let mut result = [(0, InputOperationKind::Pointer, InputDelivery::System); 9];
    let mut index = 0;
    let mut operation_index = 0;
    while operation_index < InputOperationKind::ALL.len() {
        let operation = InputOperationKind::ALL[operation_index];
        let mut route_index = 0;
        while route_index < InputDelivery::ALL.len() {
            let route = InputDelivery::ALL[route_index];
            result[index] = (pair_bit(operation, route), operation, route);
            index += 1;
            route_index += 1;
        }
        operation_index += 1;
    }
    result
}

const fn pointer_spaces(pair: InputRouteCapability) -> u32 {
    let spaces = [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ];
    let mut bits = 0;
    let mut index = 0;
    while index < spaces.len() {
        let space = spaces[index];
        if pair.accepts_pointer_space(space) {
            bits |= 1 << space_code(space);
        }
        index += 1;
    }
    bits
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use mado_pilot_runtime::{IdentityIssuer, ProviderId};

    use super::*;

    fn target() -> mado_pilot::TargetId {
        IdentityIssuer::new()
            .issue_target(ProviderId::new("capi-test"))
            .expect("issued")
    }

    fn full_size<T>() -> u32 {
        u32::try_from(size_of::<T>()).expect("ABI structure size fits u32")
    }
    fn poisoned_output<T: Versioned>() -> std::mem::MaybeUninit<T> {
        let mut output = std::mem::MaybeUninit::<T>::uninit();
        // SAFETY: every byte is initialized before the output pointer is
        // exposed, then the common first `u32` field receives the declared
        // extent expected by `Out::begin`.
        unsafe {
            output
                .as_mut_ptr()
                .cast::<u8>()
                .write_bytes(0xa5, size_of::<T>());
            output.as_mut_ptr().cast::<u32>().write(full_size::<T>());
        }
        output
    }

    fn assert_zeroed_padding<T: Versioned>(output: *const T, expected_padding: &[(usize, usize)]) {
        assert_eq!(
            T::ZEROED_PADDING,
            expected_padding,
            "{} production padding table differs from compiler-derived gaps",
            T::NAME
        );
        // SAFETY: `poisoned_output` initialized the whole allocation, and the
        // checked C output call retained that allocation and covered all bytes.
        let bytes = unsafe { std::slice::from_raw_parts(output.cast::<u8>(), size_of::<T>()) };
        for &(start, end) in expected_padding {
            assert_eq!(
                &bytes[start..end],
                vec![0; end - start],
                "{} exposed nonzero implicit padding at {start}..{end}",
                T::NAME
            );
        }
    }

    #[test]
    fn process_directed_capability_projects_the_abi_1_2_contract() {
        let capability = InputCapability::none()
            .with_pair(
                InputOperationKind::Keyboard,
                InputDelivery::ProcessDirected,
                CapabilitySupport::Unknown,
                SubmissionEvidence::InvocationOnly,
            )
            .with_permission(
                InputOperationKind::Keyboard,
                InputDelivery::ProcessDirected,
                PermissionKind::InputControl,
            );
        let projected = capability_record(
            41,
            capability.pair(InputOperationKind::Keyboard, InputDelivery::ProcessDirected),
            full_size::<madopilot_input_capability_t>(),
        );

        assert_eq!(projected.target, 41);
        assert_eq!(projected.operation, MADOPILOT_INPUT_OPERATION_KEYBOARD);
        assert_eq!(
            projected.delivery,
            MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED
        );
        assert_eq!(projected.support, MADOPILOT_CAPABILITY_UNKNOWN);
        assert_eq!(
            projected.address_scope,
            MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS
        );
        assert_eq!(
            projected.flags,
            MADOPILOT_INPUT_CAPABILITY_HAS_PERMISSION | MADOPILOT_INPUT_CAPABILITY_HAS_EVIDENCE
        );
        assert_eq!(
            projected.permission,
            MADOPILOT_PERMISSION_KIND_INPUT_CONTROL
        );
        assert_eq!(
            projected.evidence,
            MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY
        );
        assert_eq!(projected.focus_required, 0);

        let fields = capability_fields(capability);
        assert_eq!(
            fields.unknown_pairs,
            MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED
        );
        assert_eq!(fields.supported_pairs, 0);
        assert_eq!(
            fields.known_pairs & MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED,
            0,
            "unknown compatibility is not projected as known support"
        );
        assert_ne!(
            fields.known_pairs & MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM,
            0,
            "the separately unsupported system pair remains explicit"
        );
    }

    #[test]
    fn process_directed_open_and_delivery_inputs_do_not_add_system_fallback() {
        let request = madopilot_input_open_request_t {
            struct_size: full_size::<madopilot_input_open_request_t>(),
            flags: 0,
            requirement: MADOPILOT_INPUT_REQUIRED,
            reserved: 0,
            required_pairs: MADOPILOT_INPUT_PAIR_KEYBOARD_PROCESS_DIRECTED,
            preferred_pairs: 0,
        };
        // SAFETY: `request` is a live, fully initialized ABI 1.2 record.
        let converted = unsafe { open_request(&raw const request) }.expect("converted");
        assert_eq!(converted.requirement(), InputRequirement::Required);
        assert_eq!(
            converted.required(),
            &[(InputOperationKind::Keyboard, InputDelivery::ProcessDirected)]
        );

        let system_only = InputCapability::none().with_pair(
            InputOperationKind::Keyboard,
            InputDelivery::System,
            CapabilitySupport::Supported,
            SubmissionEvidence::SystemInputAdmission,
        );
        assert_eq!(
            converted
                .check(system_only)
                .expect_err("the required process pair is gated")
                .status(),
            mado_pilot::Status::Unsupported
        );

        let deliveries = [MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED];
        // SAFETY: `deliveries` is a live, aligned one-element array.
        let plan = unsafe { delivery_plan(deliveries.as_ptr(), deliveries.len()) }
            .expect("one explicit route");
        assert_eq!(plan.routes(), &[InputDelivery::ProcessDirected]);
        assert!(!plan.routes().contains(&InputDelivery::System));
    }

    #[test]
    fn process_directed_receipt_handle_retains_invocation_only_facts() {
        let target = target();
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::ProcessDirected,
            SubmissionEvidence::InvocationOnly,
            2,
        );
        let raw = handle::into_raw(InputReceiptHandle { receipt });

        assert_eq!(receipt_retain(raw), MADOPILOT_STATUS_OK);
        assert_eq!(
            receipt_release(raw),
            MADOPILOT_STATUS_OK,
            "releasing one sibling leaves the retained receipt alive"
        );

        let mut info = poisoned_output::<madopilot_input_receipt_info_t>();
        assert_eq!(receipt_info(raw, info.as_mut_ptr()), MADOPILOT_STATUS_OK);
        assert_zeroed_padding(
            info.as_ptr(),
            &[
                (
                    std::mem::offset_of!(madopilot_input_receipt_info_t, address_scope)
                        + size_of::<u32>(),
                    std::mem::offset_of!(madopilot_input_receipt_info_t, attempt_count),
                ),
                (
                    std::mem::offset_of!(madopilot_input_receipt_info_t, cleanup)
                        + size_of::<u32>(),
                    std::mem::offset_of!(madopilot_input_receipt_info_t, cleanup_released),
                ),
            ],
        );
        // SAFETY: the successful output call populated the full declared
        // structure, including its explicitly zeroed implicit padding.
        let info = unsafe { info.assume_init() };
        assert_eq!(info.target, target.get());
        assert_eq!(info.outcome, MADOPILOT_SEQUENCE_COMPLETE);
        assert_eq!(
            info.flags
                & (MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE
                    | MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE),
            MADOPILOT_INPUT_RECEIPT_HAS_SELECTED_ROUTE | MADOPILOT_INPUT_RECEIPT_HAS_EVIDENCE
        );
        assert_eq!(
            info.selected_route,
            MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED
        );
        assert_eq!(info.address_scope, MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS);
        assert_eq!(info.evidence, MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY);
        assert_eq!(info.submitted, 2);
        assert_eq!(info.attempt_count, 1);
        assert_eq!(info.flags & MADOPILOT_INPUT_RECEIPT_USED_FALLBACK, 0);

        let mut count = 0;
        assert_eq!(
            receipt_attempt_count(raw, &raw mut count),
            MADOPILOT_STATUS_OK
        );
        assert_eq!(count, 1);
        let mut attempt = poisoned_output::<madopilot_input_attempt_t>();
        assert_eq!(
            receipt_attempt_at(raw, 0, attempt.as_mut_ptr()),
            MADOPILOT_STATUS_OK
        );
        assert_zeroed_padding(
            attempt.as_ptr(),
            &[
                (
                    std::mem::offset_of!(madopilot_input_attempt_t, outcome) + size_of::<u32>(),
                    std::mem::offset_of!(madopilot_input_attempt_t, submitted),
                ),
                (
                    std::mem::offset_of!(madopilot_input_attempt_t, reserved) + size_of::<u32>(),
                    size_of::<madopilot_input_attempt_t>(),
                ),
            ],
        );
        // SAFETY: the successful output call populated the full declared
        // structure, including its explicitly zeroed implicit padding.
        let attempt = unsafe { attempt.assume_init() };
        assert_eq!(attempt.route, MADOPILOT_INPUT_DELIVERY_PROCESS_DIRECTED);
        assert_eq!(
            attempt.address_scope,
            MADOPILOT_INPUT_ADDRESS_OWNING_PROCESS
        );
        assert_eq!(
            attempt.evidence,
            MADOPILOT_SUBMISSION_EVIDENCE_INVOCATION_ONLY
        );

        assert_eq!(receipt_release(raw), MADOPILOT_STATUS_OK);
    }

    #[test]
    fn receipt_projection_keeps_target_ordinal_and_counts_above_u32() {
        let target = target();
        let submitted = usize::try_from(u64::from(u32::MAX) + 7).expect("64-bit usize");
        let submitted_u64 = u64::try_from(submitted).expect("64-bit count");
        let receipt = InputReceipt::complete(
            target,
            InputDelivery::System,
            SubmissionEvidence::SystemInputAdmission,
            submitted,
        )
        .with_cleanup(submitted + 1, submitted + 2);
        let handle = InputReceiptHandle { receipt };

        let projected = receipt_record(&handle, full_size::<madopilot_input_receipt_info_t>())
            .expect("projected");
        assert_eq!(projected.target, target.get());
        assert_eq!(projected.attempt_count, 1);
        assert_eq!(projected.submitted, submitted_u64);
        assert_eq!(projected.last_submitted, submitted_u64 - 1);
        assert_eq!(projected.cleanup_released, submitted_u64 + 1);
        assert_eq!(projected.cleanup_owed, submitted_u64 + 2);

        let attempt = attempt_record(
            handle.receipt.attempts()[0],
            full_size::<madopilot_input_attempt_t>(),
        )
        .expect("projected");
        assert_eq!(attempt.submitted, submitted_u64);
        assert_eq!(attempt.last_submitted, submitted_u64 - 1);
    }
}
