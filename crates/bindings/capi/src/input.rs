//! Native capability and input projections for the C boundary.
//!
//! The facade owns admission and delivery semantics. This module only validates
//! the C representation, converts it into that vocabulary, and projects the
//! immutable answers back into size-versioned C records.

use std::mem::{align_of, size_of};
use std::time::Duration;

use mado_pilot::{
    CapabilitySupport, CleanupBudget, CleanupState, CoordinateSpace, DeliveryPlan,
    DiagnosticCategory, Engine, FocusPolicy, GeometryPolicy, InputCapability, InputDelivery,
    InputDescriptor, InputEvent, InputFault, InputOpenRequest, InputOperationKind, InputReceipt,
    InputRequest, InputRequirement, InputSequence, Key, Modifier, PermissionKind,
    PermissionOutcome, PermissionState, Point, PointerButton, PointerGeometry, SequenceOutcome,
    TargetKind,
};

use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{FrameHandle, SessionHandle, madopilot_session_t};
use crate::engine::{TargetList, madopilot_engine_t, madopilot_target_list_t, report};
use crate::error::{self, Fault, madopilot_error_t};
use crate::operation;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_INPUT, MADOPILOT_ERROR_CATEGORY_PERMISSION,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t,
};
use crate::types::{space_code, *};
use crate::view::madopilot_str_t;
use crate::{handle, hooks};

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
        // Every event must at least name its variant. The selected variant is
        // checked against its larger mandatory prefix after this base read.
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

impl Versioned for madopilot_target_capability_t {
    const MANDATORY: usize = 56;
    const NAME: &'static str = "madopilot_target_capability_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_target_capability_t,
        struct_size,
        flags,
        target,
        kind,
        capture,
        capture_permission,
        reserved,
        input_pairs,
        focus_required,
        pointer_spaces,
        input_permission,
        reserved2,
    );

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            kind: MADOPILOT_TARGET_KIND_UNKNOWN,
            capture: MADOPILOT_CAPABILITY_UNKNOWN,
            capture_permission: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            reserved: 0,
            input_pairs: 0,
            focus_required: 0,
            pointer_spaces: 0,
            input_permission: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            reserved2: 0,
        }
    }
}

impl Versioned for madopilot_input_descriptor_t {
    const MANDATORY: usize = 40;
    const NAME: &'static str = "madopilot_input_descriptor_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_descriptor_t,
        struct_size,
        flags,
        target,
        pairs,
        focus_required,
        pointer_spaces,
        permission,
        max_events,
    );

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            pairs: 0,
            focus_required: 0,
            pointer_spaces: 0,
            permission: MADOPILOT_PERMISSION_KIND_UNSPECIFIED,
            max_events: 0,
        }
    }
}

impl Versioned for madopilot_input_receipt_t {
    const MANDATORY: usize = 64;
    const NAME: &'static str = "madopilot_input_receipt_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_input_receipt_t,
        struct_size,
        flags,
        target,
        outcome,
        delivery,
        attempted_count,
        attempted_first,
        attempted_second,
        delivered,
        last_completed,
        failure,
        cleanup,
        cleanup_released,
        cleanup_owed,
        reserved,
    );

    fn failure(struct_size: u32) -> Self {
        Self {
            struct_size,
            flags: 0,
            target: 0,
            outcome: MADOPILOT_SEQUENCE_UNEXECUTED,
            delivery: MADOPILOT_INPUT_DELIVERY_NONE,
            attempted_count: 0,
            attempted_first: MADOPILOT_INPUT_DELIVERY_NONE,
            attempted_second: MADOPILOT_INPUT_DELIVERY_NONE,
            delivered: 0,
            last_completed: 0,
            failure: MADOPILOT_INPUT_FAULT_NONE,
            cleanup: MADOPILOT_CLEANUP_NOT_NEEDED,
            cleanup_released: 0,
            cleanup_owed: 0,
            reserved: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CapabilityFields {
    pub(crate) pairs: u64,
    pub(crate) focus: u32,
    pub(crate) spaces: u32,
    pub(crate) permission: Option<PermissionKind>,
}

pub(crate) fn capability_fields(capability: InputCapability) -> CapabilityFields {
    let mut pairs = 0;
    for kind in InputOperationKind::ALL {
        for delivery in InputDelivery::ALL {
            if capability.supports(kind, delivery) {
                pairs |= pair_bit(kind, delivery);
            }
        }
    }

    let mut focus = 0;
    for delivery in InputDelivery::ALL {
        if capability.requires_focus(delivery) {
            focus |= focus_bit(delivery);
        }
    }

    let mut spaces = 0;
    for space in [
        CoordinateSpace::CapturePixels,
        CoordinateSpace::FrameNormalized,
        CoordinateSpace::TargetNormalized,
        CoordinateSpace::TargetLogical,
        CoordinateSpace::DesktopLogical,
    ] {
        if capability.accepts_pointer_space(space) {
            spaces |= 1 << space_code(space);
        }
    }

    CapabilityFields {
        pairs,
        focus,
        spaces,
        permission: capability.permission(),
    }
}

pub(crate) fn descriptor(
    target: u64,
    descriptor: &InputDescriptor,
    struct_size: u32,
) -> Result<madopilot_input_descriptor_t, Fault> {
    let fields = capability_fields(descriptor.capability());
    let (flags, permission) = match fields.permission {
        Some(permission) => (
            MADOPILOT_INPUT_DESCRIPTOR_HAS_PERMISSION,
            permission_kind_code(permission),
        ),
        None => (0, MADOPILOT_PERMISSION_KIND_UNSPECIFIED),
    };
    let max_events = u32::try_from(descriptor.limits().max_events())
        .map_err(|_| Fault::internal("the facade input event limit exceeds uint32_t"))?;

    Ok(madopilot_input_descriptor_t {
        struct_size,
        flags,
        target,
        pairs: fields.pairs,
        focus_required: fields.focus,
        pointer_spaces: fields.spaces,
        permission,
        max_events,
    })
}

pub(crate) unsafe fn open_request(
    request: *const madopilot_input_open_request_t,
) -> Result<InputOpenRequest, Fault> {
    // SAFETY: the caller of this function keeps the structure readable for the call.
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
    let known = MADOPILOT_INPUT_PAIRS_ALL;
    if request.required_pairs & !known != 0 || request.preferred_pairs & !known != 0 {
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
    for (bit, kind, delivery) in known_pairs() {
        if request.required_pairs & bit != 0 {
            converted = converted.requiring(kind, delivery);
        }
        if request.preferred_pairs & bit != 0 {
            converted = converted.preferring(kind, delivery);
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

    // SAFETY: the caller keeps the handle retained for the call.
    let Some(engine) = (unsafe { handle::borrow::<Engine>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let mut flags = 0;
    if engine.delivers_input() {
        flags |= MADOPILOT_ENGINE_DELIVERS_INPUT;
    }
    if engine.reads_permissions() {
        flags |= MADOPILOT_ENGINE_READS_PERMISSIONS;
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
    // SAFETY: the caller supplies independently writable output addresses.
    let out = match unsafe { boundary::begin_record_outputs(out_permission, out_error) } {
        Ok(out) => out,
        Err(status) => return status,
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was initialized above.
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
    let Some(engine) = (unsafe { handle::borrow::<Engine>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    let kind = permission_kind(kind)?;
    // SAFETY: the caller keeps the operation structure and cancellation handle
    // readable and retained for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let outcome = engine
        .permission(kind, context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_PERMISSION))?;
    context.commit()?;

    let value = permission_record(outcome, out.declared_size());
    // SAFETY: `out` was validated by the entry and remains writable.
    unsafe { out.commit(value) };
    Ok(())
}

pub(crate) fn target_list_capability(
    targets: *const madopilot_target_list_t,
    index: usize,
    out_capability: *mut madopilot_target_capability_t,
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
    let record = match boundary::index_within(index, targets.targets().len(), "target") {
        Ok(index) => &targets.targets()[index],
        Err(fault) => return fault.status(),
    };
    let value = target_capability_record(record, out.declared_size());

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
    // SAFETY: the caller supplies independently writable output addresses.
    let out = match unsafe { boundary::begin_record_outputs(out_descriptor, out_error) } {
        Ok(out) => out,
        Err(status) => return status,
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was initialized above.
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
    let Some(engine) = (unsafe { handle::borrow::<Engine>(engine) }) else {
        return Err(Fault::abi("`engine` is null"));
    };
    // SAFETY: as above.
    let Some(targets) = (unsafe { handle::borrow::<TargetList>(targets) }) else {
        return Err(Fault::abi("`targets` is null"));
    };
    let index = boundary::index_within(index, targets.targets().len(), "target")?;
    let target = &targets.targets()[index];
    // SAFETY: the caller keeps the operation structure and cancellation handle
    // readable and retained for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let described = engine
        .describe_input(target.facade_id(), context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_INPUT))?;
    context.commit()?;
    let value = descriptor(target.boundary_id(), &described, out.declared_size())?;

    // SAFETY: `out` was validated by the entry and remains writable.
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
    out_receipt: *mut madopilot_input_receipt_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies independently writable output addresses.
    let out = match unsafe { boundary::begin_record_outputs(out_receipt, out_error) } {
        Ok(out) => out,
        Err(status) => return status,
    };
    hooks::reach(hooks::Site::Entry);

    // SAFETY: `out_error` was initialized above.
    unsafe {
        report(
            out_error,
            run_session_send_input(session, request, operation, &out),
        )
    }
}

fn run_session_send_input(
    session: *const madopilot_session_t,
    request: *const madopilot_input_request_t,
    operation: *const madopilot_operation_t,
    out: &Out<madopilot_input_receipt_t>,
) -> Result<(), Fault> {
    // SAFETY: the caller keeps the session retained for the call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: the caller keeps the request, its borrowed arrays and strings, and
    // any source-frame handle readable and retained for the call.
    let request = unsafe { input_request(request, session) }?;
    // SAFETY: the caller keeps the operation structure and cancellation handle
    // readable and retained for the call.
    let context = unsafe { operation::context(operation) }?;
    context.admit()?;
    let receipt = session
        .session()
        .send_input(&request, context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_INPUT))?;
    let value = receipt_record(&receipt, session.boundary_target(), out.declared_size())?;
    hooks::reach(hooks::Site::AfterTemporary);

    // A receipt is already the facade operation's terminal outcome. In
    // particular, a partial receipt wins over a late cancellation or deadline,
    // so the C boundary must not replace it with a second commit check.
    // SAFETY: `out` was validated by the entry and remains writable.
    unsafe { out.commit(value) };
    Ok(())
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

fn target_capability_record(
    target: &crate::engine::TargetRecord,
    struct_size: u32,
) -> madopilot_target_capability_t {
    let capability = target.description().capability();
    let input = capability_fields(capability.input());
    let (kind_flag, kind) = match capability.kind() {
        Some(kind) => (MADOPILOT_TARGET_CAPABILITY_HAS_KIND, target_kind_code(kind)),
        None => (0, MADOPILOT_TARGET_KIND_UNKNOWN),
    };
    let (capture_permission_flag, capture_permission) = match capability.capture_permission() {
        Some(permission) => (
            MADOPILOT_TARGET_CAPABILITY_HAS_CAPTURE_PERMISSION,
            permission_kind_code(permission),
        ),
        None => (0, MADOPILOT_PERMISSION_KIND_UNSPECIFIED),
    };
    let (input_permission_flag, input_permission) = match input.permission {
        Some(permission) => (
            MADOPILOT_TARGET_CAPABILITY_HAS_INPUT_PERMISSION,
            permission_kind_code(permission),
        ),
        None => (0, MADOPILOT_PERMISSION_KIND_UNSPECIFIED),
    };

    madopilot_target_capability_t {
        struct_size,
        flags: kind_flag | capture_permission_flag | input_permission_flag,
        target: target.boundary_id(),
        kind,
        capture: capability_support_code(capability.capture()),
        capture_permission,
        reserved: 0,
        input_pairs: input.pairs,
        focus_required: input.focus,
        pointer_spaces: input.spaces,
        input_permission,
        reserved2: 0,
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
    // SAFETY: the caller keeps the event array readable for the call.
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
        // the call whenever the selected policy requires it.
        unsafe { pointer_geometry(geometry_policy, request.source_frame) }?
    } else {
        PointerGeometry::reprojected()
    };
    let cleanup = if declared!(
        request,
        madopilot_input_request_t,
        MADOPILOT_INPUT_REQUEST_HAS_CLEANUP_BUDGET
    ) {
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
        // SAFETY: `span` proved every `index * stride` is within one
        // representable object, and the caller keeps that object readable.
        let event =
            unsafe { events.cast::<u8>().add(index * stride) }.cast::<madopilot_input_event_t>();
        // SAFETY: as above. The element reader also bounds the declared prefix
        // by the array stride before copying any field.
        let event = unsafe { boundary::read_element(event, stride) }?;
        // SAFETY: any borrowed text selected by the tag remains readable for the call.
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
                covers!(
                    madopilot_input_event_t,
                    button: madopilot_pointer_button_t
                ),
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
            // SAFETY: forwarded from this function's own contract.
            let text = unsafe { crate::view::string(event.text, "event.text") }?;
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
        return Err(input_fault(InputFault::InvalidDeliveryPlan));
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
        // SAFETY: the checked span and caller contract cover this element.
        let delivery = unsafe { deliveries.add(index).read() };
        converted.push(input_delivery(delivery)?);
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
            // SAFETY: the caller keeps the frame retained for the call.
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
    receipt: &InputReceipt,
    target: u64,
    struct_size: u32,
) -> Result<madopilot_input_receipt_t, Fault> {
    let attempted = receipt.attempted();
    if attempted.len() > InputDelivery::ALL.len() {
        return Err(Fault::internal(
            "the facade receipt has more delivery attempts than ABI 1.1 can represent",
        ));
    }

    let mut flags = MADOPILOT_INPUT_RECEIPT_HAS_TARGET;
    let delivery = match receipt.delivery() {
        Some(delivery) => {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_DELIVERY;
            input_delivery_code(delivery)
        }
        None => MADOPILOT_INPUT_DELIVERY_NONE,
    };
    let last_completed = match receipt.last_completed() {
        Some(index) => {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_LAST_COMPLETED;
            narrow_receipt(index, "last completed input index")?
        }
        None => 0,
    };
    let failure = match receipt.failure() {
        Some(failure) => {
            flags |= MADOPILOT_INPUT_RECEIPT_HAS_FAILURE;
            input_fault_code(failure)
        }
        None => MADOPILOT_INPUT_FAULT_NONE,
    };
    if receipt.used_fallback() {
        flags |= MADOPILOT_INPUT_RECEIPT_USED_FALLBACK;
    }

    Ok(madopilot_input_receipt_t {
        struct_size,
        flags,
        target,
        outcome: sequence_outcome_code(receipt.outcome()),
        delivery,
        attempted_count: narrow_receipt(attempted.len(), "input attempt count")?,
        attempted_first: attempted
            .first()
            .copied()
            .map_or(MADOPILOT_INPUT_DELIVERY_NONE, input_delivery_code),
        attempted_second: attempted
            .get(1)
            .copied()
            .map_or(MADOPILOT_INPUT_DELIVERY_NONE, input_delivery_code),
        delivered: narrow_receipt(receipt.delivered(), "delivered input count")?,
        last_completed,
        failure,
        cleanup: cleanup_state_code(receipt.cleanup()),
        cleanup_released: narrow_receipt(receipt.cleanup_released(), "cleanup release count")?,
        cleanup_owed: narrow_receipt(receipt.cleanup_owed(), "cleanup owed count")?,
        reserved: 0,
    })
}

fn input_fault(fault: InputFault) -> Fault {
    let error: mado_pilot::Error = fault.into();
    Fault::from_error(&error, MADOPILOT_ERROR_CATEGORY_INPUT)
}

fn narrow_receipt(value: usize, field: &'static str) -> Result<u32, Fault> {
    u32::try_from(value)
        .map_err(|_| Fault::internal(format!("the facade {field} exceeds uint32_t")))
}

fn permission_kind(value: madopilot_permission_kind_t) -> Result<PermissionKind, Fault> {
    match value {
        MADOPILOT_PERMISSION_KIND_SCREEN_CAPTURE => Ok(PermissionKind::ScreenCapture),
        MADOPILOT_PERMISSION_KIND_INPUT_CONTROL => Ok(PermissionKind::InputControl),
        other => Err(Fault::abi(format!("unrecognized permission kind {other}"))),
    }
}

const fn permission_state_code(state: PermissionState) -> madopilot_permission_state_t {
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

fn input_delivery(value: madopilot_input_delivery_t) -> Result<InputDelivery, Fault> {
    match value {
        MADOPILOT_INPUT_DELIVERY_SYSTEM => Ok(InputDelivery::System),
        MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET => Ok(InputDelivery::BackgroundTarget),
        other => Err(Fault::abi(format!("unrecognized input delivery {other}"))),
    }
}

const fn input_delivery_code(value: InputDelivery) -> madopilot_input_delivery_t {
    match value {
        InputDelivery::System => MADOPILOT_INPUT_DELIVERY_SYSTEM,
        InputDelivery::BackgroundTarget => MADOPILOT_INPUT_DELIVERY_BACKGROUND_TARGET,
        _ => MADOPILOT_INPUT_DELIVERY_NONE,
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

const fn sequence_outcome_code(value: SequenceOutcome) -> madopilot_sequence_outcome_t {
    match value {
        SequenceOutcome::Complete => MADOPILOT_SEQUENCE_COMPLETE,
        SequenceOutcome::Partial => MADOPILOT_SEQUENCE_PARTIAL,
        SequenceOutcome::Unexecuted => MADOPILOT_SEQUENCE_UNEXECUTED,
        _ => MADOPILOT_SEQUENCE_UNEXECUTED,
    }
}

const fn cleanup_state_code(value: CleanupState) -> madopilot_cleanup_state_t {
    match value {
        CleanupState::NotNeeded => MADOPILOT_CLEANUP_NOT_NEEDED,
        CleanupState::Complete => MADOPILOT_CLEANUP_COMPLETE,
        CleanupState::Incomplete => MADOPILOT_CLEANUP_INCOMPLETE,
        CleanupState::Exhausted => MADOPILOT_CLEANUP_EXHAUSTED,
        _ => MADOPILOT_CLEANUP_INCOMPLETE,
    }
}

const fn input_fault_code(value: InputFault) -> madopilot_input_fault_t {
    match value {
        InputFault::ForeignTarget => MADOPILOT_INPUT_FAULT_FOREIGN_TARGET,
        InputFault::UnknownTarget => MADOPILOT_INPUT_FAULT_UNKNOWN_TARGET,
        InputFault::TargetLost => MADOPILOT_INPUT_FAULT_TARGET_LOST,
        InputFault::ProviderMismatch => MADOPILOT_INPUT_FAULT_PROVIDER_MISMATCH,
        InputFault::UnsupportedCombination => MADOPILOT_INPUT_FAULT_UNSUPPORTED_COMBINATION,
        InputFault::InvalidDeliveryPlan => MADOPILOT_INPUT_FAULT_INVALID_DELIVERY_PLAN,
        InputFault::DeliveryUnavailable => MADOPILOT_INPUT_FAULT_DELIVERY_UNAVAILABLE,
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
        InputFault::DeliveryFailed => MADOPILOT_INPUT_FAULT_DELIVERY_FAILED,
        _ => MADOPILOT_INPUT_FAULT_DELIVERY_FAILED,
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

const fn pair_bit(kind: InputOperationKind, delivery: InputDelivery) -> u64 {
    match (kind, delivery) {
        (InputOperationKind::Pointer, InputDelivery::System) => MADOPILOT_INPUT_PAIR_POINTER_SYSTEM,
        (InputOperationKind::Pointer, InputDelivery::BackgroundTarget) => {
            MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND
        }
        (InputOperationKind::Keyboard, InputDelivery::System) => {
            MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM
        }
        (InputOperationKind::Keyboard, InputDelivery::BackgroundTarget) => {
            MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND
        }
        (InputOperationKind::Text, InputDelivery::System) => MADOPILOT_INPUT_PAIR_TEXT_SYSTEM,
        (InputOperationKind::Text, InputDelivery::BackgroundTarget) => {
            MADOPILOT_INPUT_PAIR_TEXT_BACKGROUND
        }
        _ => 0,
    }
}

const fn focus_bit(delivery: InputDelivery) -> u32 {
    match delivery {
        InputDelivery::System => MADOPILOT_INPUT_FOCUS_SYSTEM,
        InputDelivery::BackgroundTarget => MADOPILOT_INPUT_FOCUS_BACKGROUND,
        _ => 0,
    }
}

const fn known_pairs() -> [(u64, InputOperationKind, InputDelivery); 6] {
    [
        (
            MADOPILOT_INPUT_PAIR_POINTER_SYSTEM,
            InputOperationKind::Pointer,
            InputDelivery::System,
        ),
        (
            MADOPILOT_INPUT_PAIR_POINTER_BACKGROUND,
            InputOperationKind::Pointer,
            InputDelivery::BackgroundTarget,
        ),
        (
            MADOPILOT_INPUT_PAIR_KEYBOARD_SYSTEM,
            InputOperationKind::Keyboard,
            InputDelivery::System,
        ),
        (
            MADOPILOT_INPUT_PAIR_KEYBOARD_BACKGROUND,
            InputOperationKind::Keyboard,
            InputDelivery::BackgroundTarget,
        ),
        (
            MADOPILOT_INPUT_PAIR_TEXT_SYSTEM,
            InputOperationKind::Text,
            InputDelivery::System,
        ),
        (
            MADOPILOT_INPUT_PAIR_TEXT_BACKGROUND,
            InputOperationKind::Text,
            InputDelivery::BackgroundTarget,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::ptr;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use mado_pilot_runtime::{
        CapabilitySupport, CaptureProvider, Engine, EngineWiring, IdentityIssuer, InputProvider,
        Matcher, PackageLoader, PixelExtent, PixelFormat, TargetCapability, TargetKind,
    };
    use mado_pilot_testkit::controlled_input::Behavior as InputBehavior;
    use mado_pilot_testkit::{ControlledCapture, ControlledInput, ControlledMatcher};

    use super::*;

    fn event(kind: madopilot_input_event_kind_t) -> madopilot_input_event_t {
        madopilot_input_event_t {
            struct_size: u32::try_from(size_of::<madopilot_input_event_t>())
                .expect("the event structure fits uint32_t"),
            kind,
            space: MADOPILOT_SPACE_CAPTURE_PIXELS,
            button: MADOPILOT_POINTER_BUTTON_PRIMARY,
            key: MADOPILOT_KEY_ENTER,
            key_value: 0,
            x: 4.0,
            y: 8.0,
            horizontal: 0,
            vertical: 1,
            text: madopilot_str_t::empty(),
            delay_nanos: 1,
        }
    }

    fn assert_invalid(fault: Fault) {
        assert_eq!(fault.status(), MADOPILOT_STATUS_INVALID_ARGUMENT);
    }

    fn receipt() -> madopilot_input_receipt_t {
        <madopilot_input_receipt_t as Versioned>::failure(
            u32::try_from(size_of::<madopilot_input_receipt_t>())
                .expect("the receipt structure fits uint32_t"),
        )
    }

    fn operation() -> madopilot_operation_t {
        madopilot_operation_t {
            struct_size: u32::try_from(size_of::<madopilot_operation_t>())
                .expect("the operation structure fits uint32_t"),
            flags: 0,
            deadline_nanos: 0,
            cancellation: ptr::null(),
        }
    }

    fn api() -> &'static crate::table::madopilot_api_t {
        let mut api = ptr::null();
        // SAFETY: `api` is a live, writable, correctly aligned local.
        let status = unsafe {
            crate::table::madopilot_get_api(
                crate::table::MADOPILOT_ABI_MAJOR,
                crate::table::MADOPILOT_ABI_MINOR,
                size_of::<crate::table::madopilot_api_t>(),
                &raw mut api,
            )
        };
        assert_eq!(status, MADOPILOT_STATUS_OK);

        // SAFETY: successful negotiation returns the immutable static table.
        unsafe { api.as_ref() }.expect("the negotiated table is not null")
    }

    struct InputFixture {
        engine: *mut madopilot_engine_t,
        targets: *mut madopilot_target_list_t,
        session: *mut madopilot_session_t,
        input: Arc<ControlledInput>,
    }

    /// A retained session handle borrowed by worker threads.
    ///
    /// The fixture keeps the owning reference alive until every worker joins.
    #[derive(Clone, Copy)]
    struct SharedSession(*mut madopilot_session_t);

    impl SharedSession {
        const fn as_ptr(self) -> *mut madopilot_session_t {
            self.0
        }
    }

    // SAFETY: the C ABI permits concurrent calls while each call is covered by
    // a live retained reference. The fixture owns that reference past every join.
    unsafe impl Send for SharedSession {}

    impl InputFixture {
        fn new() -> Self {
            Self::with_behavior(InputBehavior::Complete)
        }

        fn with_behavior(behavior: InputBehavior) -> Self {
            let issuer = Arc::new(IdentityIssuer::new());
            let capture = Arc::new(
                ControlledCapture::new(
                    Arc::clone(&issuer),
                    PixelExtent::new(64, 48),
                    PixelFormat::Rgba8,
                )
                .expect("a valid controlled capture"),
            );
            let input = Arc::new(ControlledInput::new(capture.target()));
            input.set_behavior(behavior);
            capture.declare(TargetCapability::new(
                TargetKind::Window,
                CapabilitySupport::Supported,
                input.capability(),
            ));
            let engine = Engine::new(EngineWiring {
                engine: issuer.engine(),
                capture: Arc::clone(&capture) as Arc<dyn CaptureProvider>,
                matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
                loader: PackageLoader::new(),
                input: Some(Arc::clone(&input) as Arc<dyn InputProvider>),
                permission: None,
            })
            .expect("compatible controlled providers");
            let engine = handle::into_raw(engine);

            let operation = operation();
            let mut targets = ptr::null_mut();
            let mut error = ptr::null_mut();
            assert_eq!(
                crate::engine::discover(
                    engine,
                    &raw const operation,
                    &raw mut targets,
                    &raw mut error,
                ),
                MADOPILOT_STATUS_OK
            );
            assert!(error.is_null());

            let input_open = madopilot_input_open_request_t {
                struct_size: u32::try_from(size_of::<madopilot_input_open_request_t>())
                    .expect("the input-open structure fits uint32_t"),
                flags: 0,
                requirement: MADOPILOT_INPUT_REQUIRED,
                reserved: 0,
                required_pairs: MADOPILOT_INPUT_PAIR_TEXT_SYSTEM,
                preferred_pairs: 0,
            };
            let open = madopilot_open_request_t {
                struct_size: u32::try_from(size_of::<madopilot_open_request_t>())
                    .expect("the open structure fits uint32_t"),
                flags: 0,
                required_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
                preferred_format: MADOPILOT_PIXEL_FORMAT_RGBA8,
            };
            let mut session = ptr::null_mut();
            assert_eq!(
                crate::capture::session_open_with_input(
                    engine,
                    targets,
                    0,
                    &raw const open,
                    &raw const input_open,
                    &raw const operation,
                    &raw mut session,
                    &raw mut error,
                ),
                MADOPILOT_STATUS_OK
            );
            assert!(error.is_null());

            Self {
                engine,
                targets,
                session,
                input,
            }
        }

        fn request<'a>(
            event: &'a madopilot_input_event_t,
            deliveries: &'a [madopilot_input_delivery_t],
        ) -> madopilot_input_request_t {
            madopilot_input_request_t {
                struct_size: u32::try_from(size_of::<madopilot_input_request_t>())
                    .expect("the input request structure fits uint32_t"),
                flags: 0,
                events: event,
                event_count: 1,
                event_stride: size_of::<madopilot_input_event_t>(),
                deliveries: deliveries.as_ptr(),
                delivery_count: deliveries.len(),
                focus_policy: MADOPILOT_FOCUS_PRESERVE,
                geometry_policy: MADOPILOT_GEOMETRY_REPROJECT_CURRENT,
                source_frame: ptr::null(),
                cleanup_max_events: 0,
                reserved: 0,
                cleanup_timeout_nanos: 0,
            }
        }

        fn request_events(
            events: &[madopilot_input_event_t],
            deliveries: &[madopilot_input_delivery_t],
        ) -> madopilot_input_request_t {
            madopilot_input_request_t {
                struct_size: u32::try_from(size_of::<madopilot_input_request_t>())
                    .expect("the input request structure fits uint32_t"),
                flags: 0,
                events: events.as_ptr(),
                event_count: events.len(),
                event_stride: size_of::<madopilot_input_event_t>(),
                deliveries: deliveries.as_ptr(),
                delivery_count: deliveries.len(),
                focus_policy: MADOPILOT_FOCUS_PRESERVE,
                geometry_policy: MADOPILOT_GEOMETRY_REPROJECT_CURRENT,
                source_frame: ptr::null(),
                cleanup_max_events: 0,
                reserved: 0,
                cleanup_timeout_nanos: 0,
            }
        }
    }

    impl Drop for InputFixture {
        fn drop(&mut self) {
            assert_eq!(
                crate::capture::session_release(self.session),
                MADOPILOT_STATUS_OK
            );
            assert_eq!(
                crate::engine::target_list_release(self.targets),
                MADOPILOT_STATUS_OK
            );
            assert_eq!(crate::engine::release(self.engine), MADOPILOT_STATUS_OK);
        }
    }

    #[test]
    fn an_entry_panic_preserves_the_retained_session() {
        let fixture = InputFixture::new();
        let api = api();
        let mut receipt = receipt();
        receipt.flags = u32::MAX;
        let mut error = ptr::null_mut();

        let status = hooks::armed(hooks::Site::Entry, || {
            // SAFETY: the session remains retained, both outputs are writable,
            // and the deliberate panic occurs before the null inputs are read.
            unsafe {
                (api.session_send_input)(
                    fixture.session,
                    ptr::null(),
                    ptr::null(),
                    &raw mut receipt,
                    &raw mut error,
                )
            }
        });

        assert_eq!(status, crate::status::MADOPILOT_STATUS_INTERNAL_PANIC);
        assert_eq!(receipt.flags, 0);
        assert!(error.is_null());

        let mut descriptor = <madopilot_input_descriptor_t as Versioned>::failure(
            u32::try_from(size_of::<madopilot_input_descriptor_t>())
                .expect("the descriptor structure fits uint32_t"),
        );
        // SAFETY: the retained session and writable output outlive the call.
        let recovered =
            unsafe { (api.session_input_descriptor)(fixture.session, &raw mut descriptor) };
        assert_eq!(recovered, MADOPILOT_STATUS_OK);
        assert!(error.is_null());
        assert_ne!(descriptor.target, 0);
    }

    #[test]
    fn a_post_delivery_panic_exposes_no_receipt_and_preserves_the_session() {
        let fixture = InputFixture::new();
        let api = api();
        let mut event = event(MADOPILOT_INPUT_EVENT_TEXT);
        event.text = madopilot_str_t::borrowed("x");
        let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
        let request = InputFixture::request(&event, &deliveries);
        let operation = operation();
        let mut receipt = receipt();
        receipt.flags = u32::MAX;
        receipt.delivered = u32::MAX;
        let mut error = ptr::null_mut();

        let status = hooks::armed(hooks::Site::AfterTemporary, || {
            // SAFETY: every handle, structure, and borrowed view remains live.
            unsafe {
                (api.session_send_input)(
                    fixture.session,
                    &raw const request,
                    &raw const operation,
                    &raw mut receipt,
                    &raw mut error,
                )
            }
        });

        assert_eq!(status, crate::status::MADOPILOT_STATUS_INTERNAL_PANIC);
        assert_eq!(receipt.flags, 0);
        assert_eq!(receipt.delivered, 0);
        assert!(error.is_null());
        assert_eq!(
            fixture.input.delivered().len(),
            1,
            "the panic was reached only after the adapter returned"
        );

        let mut descriptor = <madopilot_input_descriptor_t as Versioned>::failure(
            u32::try_from(size_of::<madopilot_input_descriptor_t>())
                .expect("the descriptor structure fits uint32_t"),
        );
        // SAFETY: the retained session and writable output outlive the call.
        let recovered =
            unsafe { (api.session_input_descriptor)(fixture.session, &raw mut descriptor) };
        assert_eq!(recovered, MADOPILOT_STATUS_OK);
        assert!(error.is_null());
    }

    #[test]
    fn an_invalid_event_array_never_reaches_the_controller() {
        let fixture = InputFixture::new();
        let event = event(MADOPILOT_INPUT_EVENT_TEXT);
        let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
        let mut request = InputFixture::request(&event, &deliveries);
        request.event_stride = <madopilot_input_event_t as boundary::Input>::MANDATORY - 1;
        let operation = operation();
        let mut receipt = receipt();
        receipt.flags = u32::MAX;
        receipt.delivered = u32::MAX;
        let mut error = ptr::null_mut();

        let status = session_send_input(
            fixture.session,
            &raw const request,
            &raw const operation,
            &raw mut receipt,
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
        assert_eq!(receipt.flags, 0);
        assert_eq!(receipt.delivered, 0);
        assert!(!error.is_null());
        assert!(fixture.input.admitted().is_empty());
        assert!(fixture.input.delivered().is_empty());
        assert_eq!(error::release(error), MADOPILOT_STATUS_OK);
    }

    #[test]
    fn a_complete_sequence_is_successful_receipt_data() {
        let fixture = InputFixture::new();
        let mut event = event(MADOPILOT_INPUT_EVENT_TEXT);
        event.text = madopilot_str_t::borrowed("x");
        let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
        let request = InputFixture::request(&event, &deliveries);
        let operation = operation();
        let mut receipt = receipt();
        let mut error = ptr::null_mut();

        let status = session_send_input(
            fixture.session,
            &raw const request,
            &raw const operation,
            &raw mut receipt,
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_OK);
        assert!(error.is_null());
        assert_eq!(receipt.outcome, MADOPILOT_SEQUENCE_COMPLETE);
        assert_eq!(receipt.delivery, MADOPILOT_INPUT_DELIVERY_SYSTEM);
        assert_eq!(receipt.delivered, 1);
        assert_eq!(receipt.last_completed, 0);
        assert_ne!(
            receipt.flags & MADOPILOT_INPUT_RECEIPT_HAS_LAST_COMPLETED,
            0
        );
        assert_eq!(receipt.failure, MADOPILOT_INPUT_FAULT_NONE);
        assert_eq!(fixture.input.admitted().len(), 1);
        assert_eq!(fixture.input.delivered().len(), 1);
    }

    #[test]
    fn concurrent_c_calls_keep_each_input_sequence_contiguous() {
        const WORKERS: usize = 8;
        const CALLS_PER_WORKER: usize = 16;

        let fixture = InputFixture::new();
        let send = api().session_send_input;
        let session = SharedSession(fixture.session);
        let start = Arc::new(Barrier::new(WORKERS));
        let mut workers = Vec::with_capacity(WORKERS);

        for _ in 0..WORKERS {
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                let events = [
                    event(MADOPILOT_INPUT_EVENT_KEY_PRESS),
                    event(MADOPILOT_INPUT_EVENT_KEY_RELEASE),
                ];
                let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
                let request = InputFixture::request_events(&events, &deliveries);
                let operation = operation();
                start.wait();

                for _ in 0..CALLS_PER_WORKER {
                    let mut receipt = receipt();
                    let mut error = ptr::null_mut();
                    // SAFETY: the fixture retains the session until this worker
                    // joins, and every request, operation, and output is local to
                    // this worker for the duration of the call.
                    let status = unsafe {
                        send(
                            session.as_ptr(),
                            &raw const request,
                            &raw const operation,
                            &raw mut receipt,
                            &raw mut error,
                        )
                    };

                    assert_eq!(status, MADOPILOT_STATUS_OK);
                    assert!(error.is_null());
                    assert_eq!(receipt.outcome, MADOPILOT_SEQUENCE_COMPLETE);
                    assert_eq!(receipt.delivered, 2);
                }
            }));
        }

        for worker in workers {
            worker.join().expect("every C caller completed");
        }

        let delivered = fixture.input.delivered();
        assert_eq!(delivered.len(), WORKERS * CALLS_PER_WORKER * 2);
        for sequence in delivered.chunks_exact(2) {
            assert!(
                matches!(sequence[0].event, InputEvent::KeyPress(Key::Enter)),
                "a second sequence interleaved before the first sequence pressed its key"
            );
            assert!(
                matches!(sequence[1].event, InputEvent::KeyRelease(Key::Enter)),
                "a second sequence interleaved before the first sequence released its key"
            );
        }
    }

    #[test]
    fn a_zero_completed_event_partial_is_successful_receipt_data() {
        let fixture = InputFixture::with_behavior(InputBehavior::FailAfter {
            delivered: 0,
            fault: InputFault::DeliveryFailed,
        });
        let mut event = event(MADOPILOT_INPUT_EVENT_TEXT);
        event.text = madopilot_str_t::borrowed("x");
        let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
        let request = InputFixture::request(&event, &deliveries);
        let operation = operation();
        let mut receipt = receipt();
        let mut error = ptr::null_mut();

        let status = session_send_input(
            fixture.session,
            &raw const request,
            &raw const operation,
            &raw mut receipt,
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_OK);
        assert!(error.is_null());
        assert_eq!(receipt.outcome, MADOPILOT_SEQUENCE_PARTIAL);
        assert_eq!(receipt.delivered, 0);
        assert_eq!(receipt.last_completed, 0);
        assert_eq!(
            receipt.flags & MADOPILOT_INPUT_RECEIPT_HAS_LAST_COMPLETED,
            0
        );
        assert_eq!(receipt.failure, MADOPILOT_INPUT_FAULT_DELIVERY_FAILED);
        assert_eq!(fixture.input.admitted().len(), 1);
        assert!(fixture.input.delivered().is_empty());
    }

    #[test]
    fn an_unexecuted_sequence_is_successful_receipt_data() {
        let fixture =
            InputFixture::with_behavior(InputBehavior::Unexecuted(InputFault::PolicyRefused));
        let mut event = event(MADOPILOT_INPUT_EVENT_TEXT);
        event.text = madopilot_str_t::borrowed("x");
        let deliveries = [MADOPILOT_INPUT_DELIVERY_SYSTEM];
        let request = InputFixture::request(&event, &deliveries);
        let operation = operation();
        let mut receipt = receipt();
        let mut error = ptr::null_mut();

        let status = session_send_input(
            fixture.session,
            &raw const request,
            &raw const operation,
            &raw mut receipt,
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_OK);
        assert!(error.is_null());
        assert_eq!(receipt.outcome, MADOPILOT_SEQUENCE_UNEXECUTED);
        assert_eq!(receipt.delivered, 0);
        assert_eq!(receipt.failure, MADOPILOT_INPUT_FAULT_POLICY_REFUSED);
        assert_eq!(fixture.input.admitted().len(), 1);
        assert!(fixture.input.delivered().is_empty());
    }

    #[test]
    fn a_missing_receipt_is_reported_without_controller_work() {
        let fixture = InputFixture::new();
        let mut error = ptr::null_mut();

        let status = session_send_input(
            fixture.session,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
        assert!(!error.is_null());
        assert!(fixture.input.admitted().is_empty());
        assert!(fixture.input.delivered().is_empty());
        assert_eq!(error::release(error), MADOPILOT_STATUS_OK);
    }

    #[test]
    fn an_event_stride_below_the_base_prefix_is_rejected_before_reading() {
        let event = event(MADOPILOT_INPUT_EVENT_DELAY);

        // SAFETY: `event` remains readable for the call; the deliberately short
        // stride is data for the validator and is rejected before pointer math.
        let fault = unsafe {
            input_events(
                &raw const event,
                1,
                <madopilot_input_event_t as boundary::Input>::MANDATORY - 1,
                1,
            )
        }
        .expect_err("a short stride is invalid");

        assert_invalid(fault);
    }

    #[test]
    fn an_event_variant_requires_its_own_prefix() {
        let mut event = event(MADOPILOT_INPUT_EVENT_KEY_PRESS);
        event.struct_size = u32::try_from(covers!(
            madopilot_input_event_t,
            key: madopilot_key_t
        ))
        .expect("the prefix fits uint32_t");

        // SAFETY: `event` and its full array stride remain readable for the call.
        let fault =
            unsafe { input_events(&raw const event, 1, size_of::<madopilot_input_event_t>(), 1) }
                .expect_err("a key event without key_value is invalid");

        assert_invalid(fault);
    }

    #[test]
    fn invalid_event_text_is_rejected_as_utf8() {
        let bytes = [0xff];
        let mut event = event(MADOPILOT_INPUT_EVENT_TEXT);
        event.text = madopilot_str_t {
            data: bytes.as_ptr().cast(),
            len: bytes.len(),
        };

        // SAFETY: the event and its text bytes remain readable for the call.
        let fault =
            unsafe { input_events(&raw const event, 1, size_of::<madopilot_input_event_t>(), 1) }
                .expect_err("invalid UTF-8 is rejected");

        assert_invalid(fault);
    }

    #[test]
    fn nonfinite_pointer_coordinates_are_rejected() {
        let mut event = event(MADOPILOT_INPUT_EVENT_POINTER_MOVE);
        event.x = f64::NAN;

        // SAFETY: `event` remains readable for the call.
        let fault =
            unsafe { input_events(&raw const event, 1, size_of::<madopilot_input_event_t>(), 1) }
                .expect_err("NaN cannot cross into a point");

        assert_invalid(fault);
    }

    #[test]
    fn a_repeated_delivery_is_rejected_before_admission() {
        let deliveries = [
            MADOPILOT_INPUT_DELIVERY_SYSTEM,
            MADOPILOT_INPUT_DELIVERY_SYSTEM,
        ];

        // SAFETY: the delivery array remains readable for the call.
        let fault = unsafe { delivery_plan(deliveries.as_ptr(), deliveries.len()) }
            .expect_err("a repeated mechanism is ambiguous");

        assert_invalid(fault);
    }

    #[test]
    fn a_pre_receipt_failure_initializes_the_receipt_and_owns_its_error() {
        let mut receipt = madopilot_input_receipt_t {
            struct_size: u32::try_from(size_of::<madopilot_input_receipt_t>())
                .expect("the receipt structure fits uint32_t"),
            flags: u32::MAX,
            target: u64::MAX,
            outcome: MADOPILOT_SEQUENCE_COMPLETE,
            delivery: MADOPILOT_INPUT_DELIVERY_SYSTEM,
            attempted_count: u32::MAX,
            attempted_first: MADOPILOT_INPUT_DELIVERY_SYSTEM,
            attempted_second: MADOPILOT_INPUT_DELIVERY_SYSTEM,
            delivered: u32::MAX,
            last_completed: u32::MAX,
            failure: MADOPILOT_INPUT_FAULT_DELIVERY_FAILED,
            cleanup: MADOPILOT_CLEANUP_EXHAUSTED,
            cleanup_released: u32::MAX,
            cleanup_owed: u32::MAX,
            reserved: u32::MAX,
        };
        let mut error = ptr::null_mut();

        let status = session_send_input(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            &raw mut receipt,
            &raw mut error,
        );

        assert_eq!(status, MADOPILOT_STATUS_INVALID_ARGUMENT);
        assert_eq!(receipt.flags, 0);
        assert_eq!(receipt.target, 0);
        assert_eq!(receipt.outcome, MADOPILOT_SEQUENCE_UNEXECUTED);
        assert_eq!(receipt.delivered, 0);
        assert_eq!(receipt.cleanup, MADOPILOT_CLEANUP_NOT_NEEDED);
        assert!(!error.is_null(), "the valid error output owns the refusal");
        assert_eq!(error::release(error), MADOPILOT_STATUS_OK);
    }
}
