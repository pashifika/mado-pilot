//! Pull-only projection of the facade's template-query authority.
//!
//! C references share one Rust query. Its only cache holds one immutable terminal
//! projection; pending polls allocate no result storage. Results retain the Rust
//! terminal and a non-owning mapping observer, never a query or parent session.
//! No cache initialization spans Rust poll, wait, cancel, or frame mapping.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use mado_pilot::{
    ChangeDetectionPolicy, Error, MappingObserver, PreparedTemplate, Status, TemplateAnalysisRate,
    TemplateOverload, TemplateQuery, TemplateQueryOutcome, TemplateQueryProgress,
    TemplateStability, TemplateTerminalOutcome, TemplateWatchRequest, TemplateWorkDisposition,
    TransformSnapshot,
};

use crate::assets::madopilot_template_t;
use crate::boundary::{self, Out, Versioned, covers, declared, inputs, prefixes};
use crate::capture::{
    FrameHandle, SessionHandle, madopilot_frame_t, madopilot_session_t, rect, stamp,
};
use crate::engine::{EngineHandle, madopilot_engine_t, report};
use crate::error::{self, Fault, madopilot_error_t};
use crate::handle::opaque;
use crate::layout::struct_size;
use crate::status::{
    MADOPILOT_ERROR_CATEGORY_ABI, MADOPILOT_ERROR_CATEGORY_ASSET, MADOPILOT_ERROR_CATEGORY_CAPTURE,
    MADOPILOT_ERROR_CATEGORY_INPUT, MADOPILOT_ERROR_CATEGORY_OPERATION,
    MADOPILOT_ERROR_CATEGORY_UNSPECIFIED, MADOPILOT_ERROR_CATEGORY_VISION,
    MADOPILOT_STATUS_INVALID_ARGUMENT, MADOPILOT_STATUS_OK, madopilot_status_t,
};
use crate::types::{
    MADOPILOT_FIND_HAS_REGION, MADOPILOT_MATCH_HAS_MAX_RESULTS, MADOPILOT_MATCH_HAS_MIN_SCORE,
    MADOPILOT_MATCH_HAS_SUPPRESSION, MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS,
    MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA, MADOPILOT_TEMPLATE_OVERLOAD_QUEUE_EXPIRED,
    MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME, MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED,
    MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED, MADOPILOT_TEMPLATE_QUERY_OUTCOME_FAILED,
    MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED, MADOPILOT_TEMPLATE_QUERY_OUTCOME_OVERLOADED,
    MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED,
    MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED, MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST,
    MADOPILOT_TEMPLATE_QUERY_RESULT_INFO_SIZE_V1_6, MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6,
    MADOPILOT_TEMPLATE_QUERY_STATE_PENDING, MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL,
    MADOPILOT_TEMPLATE_SCHEDULER_DESCRIPTOR_SIZE_V1_6, MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE,
    MADOPILOT_TEMPLATE_STABILITY_DURATION, MADOPILOT_TEMPLATE_STABILITY_IMMEDIATE,
    MADOPILOT_TEMPLATE_WATCH_HAS_REGION, MADOPILOT_TEMPLATE_WATCH_OPTIONS_SIZE_V1_6,
    MADOPILOT_TRANSFORM_COVERS_TARGET, MADOPILOT_TRANSFORM_HAS_TARGET_PLACEMENT,
    MADOPILOT_TRANSFORM_SNAPSHOT_SIZE_V1_6, madopilot_clip_policy_t, madopilot_find_request_t,
    madopilot_frame_stamp_t, madopilot_match_options_t, madopilot_match_t, madopilot_operation_t,
    madopilot_template_query_result_info_t, madopilot_template_query_snapshot_t,
    madopilot_template_scheduler_descriptor_t, madopilot_template_watch_options_t,
    madopilot_transform_snapshot_t, suppression_code,
};
use crate::view::madopilot_str_t;
use crate::{handle, hooks, matching, operation};

opaque! {
    /// Shared ownership of one pull-based template query.
    madopilot_template_query_t => TemplateQueryHandle
}

opaque! {
    /// Immutable terminal facts, independent of query and parent lifetimes.
    madopilot_template_query_result_t => TemplateQueryResultHandle
}

#[derive(Debug)]
pub(crate) struct TemplateQueryHandle {
    query: TemplateQuery,
    mapping_observer: MappingObserver,
    terminal: OnceLock<Arc<TemplateQueryResultHandle>>,
}

impl TemplateQueryHandle {
    fn terminal(&self, outcome: Arc<TemplateTerminalOutcome>) -> Arc<TemplateQueryResultHandle> {
        Arc::clone(self.terminal.get_or_init(|| {
            Arc::new(TemplateQueryResultHandle {
                query_id: self.query.id().get(),
                outcome,
                mapping_observer: self.mapping_observer.clone(),
            })
        }))
    }
}

#[derive(Debug)]
pub(crate) struct TemplateQueryResultHandle {
    query_id: u64,
    outcome: Arc<TemplateTerminalOutcome>,
    mapping_observer: MappingObserver,
}

inputs! {
    impl Input for madopilot_template_watch_options_t {
        const MANDATORY: usize = MADOPILOT_TEMPLATE_WATCH_OPTIONS_SIZE_V1_6 as usize;
        const NAME: &'static str = "madopilot_template_watch_options_t";
        const PREFIXES: &'static [usize] = prefixes!(
            madopilot_template_watch_options_t,
            struct_size, flags, match_options, region, clip_policy, minimum_interval_nanos,
            stability_kind, stability_observations, stability_duration_nanos, change_policy,
            reserved,
        );
        const PRESENCE: &'static [(u32, usize)] = &[(
            MADOPILOT_TEMPLATE_WATCH_HAS_REGION,
            covers!(madopilot_template_watch_options_t, clip_policy: madopilot_clip_policy_t),
        )];

        fn defaults() -> Self {
            Self::cleared(0)
        }

        fn presence_bits(&self) -> u32 {
            self.flags
        }
    }
}

impl Versioned for madopilot_template_scheduler_descriptor_t {
    const MANDATORY: usize = MADOPILOT_TEMPLATE_SCHEDULER_DESCRIPTOR_SIZE_V1_6 as usize;
    const NAME: &'static str = "madopilot_template_scheduler_descriptor_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_template_scheduler_descriptor_t,
        struct_size,
        flags,
        max_engine_queries,
        max_active_sessions,
        max_session_queries,
        max_in_flight_analyses,
        latest_pending_frames_per_query,
        max_mapped_cache_entries,
        mapped_cache_bytes,
        eligible_queue_expiry_nanos,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

impl Versioned for madopilot_template_query_snapshot_t {
    const MANDATORY: usize = MADOPILOT_TEMPLATE_QUERY_SNAPSHOT_SIZE_V1_6 as usize;
    const NAME: &'static str = "madopilot_template_query_snapshot_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_template_query_snapshot_t,
        struct_size,
        state,
        flags,
        confirmed_observations,
        query_id,
        generation,
        confirmed_duration_nanos,
        pending_count,
        in_flight_count,
        last_frame,
        admitted,
        skipped_change,
        deferred_rate,
        coalesced,
        superseded,
        rejected,
        queue_expired,
        completed,
        failed,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

impl Versioned for madopilot_transform_snapshot_t {
    const MANDATORY: usize = MADOPILOT_TRANSFORM_SNAPSHOT_SIZE_V1_6 as usize;
    const NAME: &'static str = "madopilot_transform_snapshot_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_transform_snapshot_t,
        struct_size,
        flags,
        geometry,
        width,
        height,
        desktop_origin_x,
        desktop_origin_y,
        logical_width,
        logical_height,
        target_scale_x,
        target_scale_y,
        desktop_scale_x,
        desktop_scale_y,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

impl Versioned for madopilot_template_query_result_info_t {
    const MANDATORY: usize = MADOPILOT_TEMPLATE_QUERY_RESULT_INFO_SIZE_V1_6 as usize;
    const NAME: &'static str = "madopilot_template_query_result_info_t";
    const PREFIXES: &'static [usize] = prefixes!(
        madopilot_template_query_result_info_t,
        struct_size,
        outcome,
        status,
        overload,
        query_id,
        target,
        source,
        match_count,
        template_id,
        backend_id,
        backend_version,
        options,
        effective_region,
        confirmed_observations,
        confirmed_duration_nanos,
        transform,
    );
    const ZEROED_PADDING: &'static [(usize, usize)] = &[];

    fn failure(struct_size: u32) -> Self {
        Self::cleared(struct_size)
    }
}

pub(crate) fn engine_template_scheduler_descriptor(
    engine: *const madopilot_engine_t,
    out_descriptor: *mut madopilot_template_scheduler_descriptor_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies the declared writable output extent.
    let out = match unsafe { Out::begin(out_descriptor) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller retains the engine throughout this call.
    let Some(engine) = (unsafe { handle::borrow::<EngineHandle>(engine) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let descriptor = engine.template_scheduler();
    // SAFETY: the output was validated and initialized above.
    unsafe {
        out.commit(madopilot_template_scheduler_descriptor_t {
            struct_size: out.declared_size(),
            flags: 0,
            max_engine_queries: descriptor.max_engine_queries(),
            max_active_sessions: descriptor.max_active_sessions(),
            max_session_queries: descriptor.max_session_queries(),
            max_in_flight_analyses: descriptor.max_in_flight_analyses(),
            latest_pending_frames_per_query: descriptor.latest_pending_frames_per_query(),
            max_mapped_cache_entries: descriptor.max_mapped_cache_entries(),
            mapped_cache_bytes: descriptor.mapped_cache_bytes(),
            eligible_queue_expiry_nanos: nanos(descriptor.eligible_queue_expiry()),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn session_start_template_watch(
    session: *const madopilot_session_t,
    tmpl: *const madopilot_template_t,
    options: *const madopilot_template_watch_options_t,
    query_operation: *const madopilot_operation_t,
    out_query: *mut *mut madopilot_template_query_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: each output independently satisfies its writable storage contract.
    if let Err(status) = unsafe { boundary::begin_outputs(out_query, "out_query", out_error) } {
        return status;
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: out_error was validated above.
    unsafe {
        report(
            out_error,
            run_start(session, tmpl, options, query_operation, out_query),
        )
    }
}

fn run_start(
    session: *const madopilot_session_t,
    tmpl: *const madopilot_template_t,
    options: *const madopilot_template_watch_options_t,
    query_operation: *const madopilot_operation_t,
    out_query: *mut *mut madopilot_template_query_t,
) -> Result<(), Fault> {
    // SAFETY: the caller retains both handles for the entire start call.
    let Some(session) = (unsafe { handle::borrow::<SessionHandle>(session) }) else {
        return Err(Fault::abi("`session` is null"));
    };
    // SAFETY: as above; wrong-type or stale opaque handles are caller violations.
    let Some(template) = (unsafe { handle::borrow::<PreparedTemplate>(tmpl) }) else {
        return Err(Fault::abi("`tmpl` is null"));
    };
    // SAFETY: input records and their pointees remain readable for this call.
    let options = unsafe { boundary::read_input(options) }?;
    // SAFETY: the operation and any cancellation handle remain live for this call.
    let context = unsafe { operation::context(query_operation) }?;
    let request = watch_request(options, template, context)?;
    let mapping_observer = session.session().mapping_observer();
    let query = session
        .session()
        .start_template_watch(request)
        .map_err(|error| runtime_fault(&error))?;
    let payload = TemplateQueryHandle {
        query,
        mapping_observer,
        terminal: OnceLock::new(),
    };
    hooks::reach(hooks::Site::AfterTemporary);
    // Rust has already arbitrated publication. A second operation check here
    // would replace its winner; an unwinding unpublished owner simply drops.
    // SAFETY: the entry preflighted out_query before any query was started.
    unsafe { out_query.write(handle::into_raw(payload)) };
    Ok(())
}

fn watch_request(
    options: madopilot_template_watch_options_t,
    template: &PreparedTemplate,
    context: operation::Context,
) -> Result<TemplateWatchRequest, Fault> {
    if options.flags & !MADOPILOT_TEMPLATE_WATCH_HAS_REGION != 0 {
        return Err(Fault::abi("unknown template watch flags"));
    }
    if options.reserved != 0 {
        return Err(Fault::abi("template watch reserved bits must be zero"));
    }
    let has_region = declared!(
        options,
        madopilot_template_watch_options_t,
        MADOPILOT_TEMPLATE_WATCH_HAS_REGION
    );
    let find = madopilot_find_request_t {
        struct_size: struct_size::<madopilot_find_request_t>(),
        flags: if has_region {
            MADOPILOT_FIND_HAS_REGION
        } else {
            0
        },
        frame: std::ptr::null(),
        tmpl: std::ptr::null(),
        options: options.match_options,
        region: options.region,
        clip_policy: options.clip_policy,
    };
    let matching = matching::resolve_options(&find, template)?;
    let region = if has_region {
        Some(matching::region_selection(&find)?)
    } else {
        None
    };
    let rate = if options.minimum_interval_nanos == 0 {
        TemplateAnalysisRate::unrestricted()
    } else {
        TemplateAnalysisRate::at_most_every(Duration::from_nanos(options.minimum_interval_nanos))
            .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ABI))?
    };
    let stability = match options.stability_kind {
        MADOPILOT_TEMPLATE_STABILITY_IMMEDIATE
            if options.stability_observations == 0 && options.stability_duration_nanos == 0 =>
        {
            TemplateStability::immediate()
        }
        MADOPILOT_TEMPLATE_STABILITY_CONSECUTIVE if options.stability_duration_nanos == 0 => {
            TemplateStability::consecutive(options.stability_observations)
                .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ABI))?
        }
        MADOPILOT_TEMPLATE_STABILITY_DURATION if options.stability_observations == 0 => {
            TemplateStability::duration(Duration::from_nanos(options.stability_duration_nanos))
                .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_ABI))?
        }
        _ => {
            return Err(Fault::abi(
                "invalid template stability kind or active fields",
            ));
        }
    };
    let policy = match options.change_policy {
        MADOPILOT_TEMPLATE_CHANGE_ANALYSIS_ALWAYS => ChangeDetectionPolicy::AnalysisAlways,
        MADOPILOT_TEMPLATE_CHANGE_EXACT_RGBA => ChangeDetectionPolicy::ExactRgba,
        _ => return Err(Fault::abi("unknown template change policy")),
    };
    let mut request = TemplateWatchRequest::new(template.clone(), matching, context.into_inner())
        .with_rate(rate)
        .with_stability(stability)
        .with_change_policy(policy);
    if let Some(region) = region {
        request = request.with_region(region);
    }
    Ok(request)
}

pub(crate) fn template_query_retain(
    query: *const madopilot_template_query_t,
) -> madopilot_status_t {
    // SAFETY: null is permitted; otherwise the caller holds a live reference.
    unsafe { handle::retain::<TemplateQueryHandle>(query) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_release(query: *mut madopilot_template_query_t) -> madopilot_status_t {
    // SAFETY: the caller gives up one owned reference, or passes null.
    unsafe { handle::release::<TemplateQueryHandle>(query) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_poll(
    query: *const madopilot_template_query_t,
    out_snapshot: *mut madopilot_template_query_snapshot_t,
    out_result: *mut *mut madopilot_template_query_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // Preflight all three independently, with required-output signature order.
    // SAFETY: the caller supplies independent writable extents for each output.
    let snapshot = unsafe { Out::begin(out_snapshot) };
    // SAFETY: as above.
    let result = unsafe { boundary::begin_handle_out(out_result, "out_result") };
    // SAFETY: as above; out_error alone is optional.
    let error = unsafe { boundary::begin_error_out(out_error) };
    let out = match (snapshot, result, error) {
        (Ok(out), Ok(()), Ok(())) => out,
        (Err(fault), _, Ok(())) | (Ok(_), Err(fault), Ok(())) => {
            // SAFETY: the error output was accepted and initialized.
            return unsafe { error::emit(out_error, fault) };
        }
        (Err(fault), _, Err(_)) | (Ok(_), Err(fault), Err(_)) | (Ok(_), Ok(()), Err(fault)) => {
            return fault.status();
        }
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller keeps its own query reference throughout the call.
    let Some(query) = (unsafe { handle::borrow::<TemplateQueryHandle>(query) }) else {
        // SAFETY: out_error was accepted above.
        return unsafe { error::emit(out_error, Fault::abi("`query` is null")) };
    };
    match query.query.poll() {
        TemplateQueryOutcome::Pending(progress) => {
            // SAFETY: out remains the validated writable snapshot.
            unsafe { out.commit(pending_snapshot(progress, out.declared_size())) };
        }
        TemplateQueryOutcome::Terminal(outcome) => {
            let result = query.terminal(outcome);
            let mut snapshot = madopilot_template_query_snapshot_t::cleared(out.declared_size());
            snapshot.state = MADOPILOT_TEMPLATE_QUERY_STATE_TERMINAL;
            snapshot.query_id = result.query_id;
            hooks::reach(hooks::Site::AfterTemporary);
            // No fallible or panic-capable work separates the two publications.
            // SAFETY: both outputs were independently validated before polling.
            unsafe {
                out.commit(snapshot);
                out_result.write(
                    Arc::into_raw(result)
                        .cast::<madopilot_template_query_result_t>()
                        .cast_mut(),
                );
            }
        }
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_wait(
    query: *const madopilot_template_query_t,
    wait_operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_template_query_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies independent writable handle outputs.
    if let Err(status) = unsafe { boundary::begin_outputs(out_result, "out_result", out_error) } {
        return status;
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: out_error was accepted above.
    unsafe { report(out_error, run_wait(query, wait_operation, out_result)) }
}

fn run_wait(
    query: *const madopilot_template_query_t,
    wait_operation: *const madopilot_operation_t,
    out_result: *mut *mut madopilot_template_query_result_t,
) -> Result<(), Fault> {
    // SAFETY: the caller retains the query throughout this wait.
    let Some(query) = (unsafe { handle::borrow::<TemplateQueryHandle>(query) }) else {
        return Err(Fault::abi("`query` is null"));
    };
    // SAFETY: the operation and cancellation reference remain readable/live.
    let context = unsafe { operation::context(wait_operation) }?;
    // Always delegate, even when cached: Rust checks caller interruption before
    // an existing terminal, and owns the last check when waiting succeeds.
    let outcome = query
        .query
        .wait(context.inner())
        .map_err(error::facade(MADOPILOT_ERROR_CATEGORY_OPERATION))?;
    let result = query.terminal(outcome);
    hooks::reach(hooks::Site::AfterTemporary);
    // SAFETY: the entry initialized this output before waiting.
    unsafe {
        out_result.write(
            Arc::into_raw(result)
                .cast::<madopilot_template_query_result_t>()
                .cast_mut(),
        )
    };
    Ok(())
}

pub(crate) fn template_query_cancel(
    query: *const madopilot_template_query_t,
    out_result: *mut *mut madopilot_template_query_result_t,
    out_error: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // An invalid output must not compete cancellation, including out_error.
    // SAFETY: each output independently satisfies its writable storage contract.
    if let Err(status) = unsafe { boundary::begin_outputs(out_result, "out_result", out_error) } {
        return status;
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller retains the query across cancellation.
    let Some(query) = (unsafe { handle::borrow::<TemplateQueryHandle>(query) }) else {
        // SAFETY: out_error was accepted above.
        return unsafe { error::emit(out_error, Fault::abi("`query` is null")) };
    };
    let outcome = query.query.cancel();
    let result = query.terminal(outcome);
    hooks::reach(hooks::Site::AfterTemporary);
    // SAFETY: out_result was initialized before competing terminal authority.
    unsafe {
        out_result.write(
            Arc::into_raw(result)
                .cast::<madopilot_template_query_result_t>()
                .cast_mut(),
        )
    };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_retain(
    result: *const madopilot_template_query_result_t,
) -> madopilot_status_t {
    // SAFETY: the caller holds a reference for this call, or passes null.
    unsafe { handle::retain::<TemplateQueryResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_release(
    result: *mut madopilot_template_query_result_t,
) -> madopilot_status_t {
    // SAFETY: the caller gives up one owned reference, or passes null.
    unsafe { handle::release::<TemplateQueryResultHandle>(result) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_info(
    result: *const madopilot_template_query_result_t,
    out_info: *mut madopilot_template_query_result_info_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies the declared writable record extent.
    let out = match unsafe { Out::begin(out_info) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the result remains retained for the call and borrowed-view use.
    let Some(result) = (unsafe { handle::borrow::<TemplateQueryResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let info = match terminal_info(result, out.declared_size()) {
        Ok(info) => info,
        Err(fault) => return fault.status(),
    };
    // SAFETY: out was validated; views borrow only the retained result.
    unsafe { out.commit(info) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_match_at(
    result: *const madopilot_template_query_result_t,
    index: usize,
    out_match: *mut madopilot_match_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies the declared writable match extent.
    let out = match unsafe { Out::begin(out_match) } {
        Ok(out) => out,
        Err(fault) => return fault.status(),
    };
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller retains the result for this call and borrowed-view use.
    let Some(result) = (unsafe { handle::borrow::<TemplateQueryResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let TemplateTerminalOutcome::Matched(matched) = result.outcome.as_ref() else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let matches = matched.result().matches();
    let index = match boundary::index_within(index, matches.len(), "match") {
        Ok(index) => index,
        Err(fault) => return fault.status(),
    };
    let found = &matches[index];
    // SAFETY: out was validated; the template view borrows the result's array.
    unsafe {
        out.commit(madopilot_match_t {
            struct_size: out.declared_size(),
            flags: 0,
            score: found.score(),
            template_id: madopilot_str_t::borrowed(found.template().as_str()),
            bounds: rect(found.bounds()),
        });
    }
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_frame(
    result: *const madopilot_template_query_result_t,
    out_frame: *mut *mut madopilot_frame_t,
) -> madopilot_status_t {
    // SAFETY: the caller supplies writable owned-frame output storage.
    if let Err(fault) = unsafe { boundary::begin_handle_out(out_frame, "out_frame") } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller retains the result for this call.
    let Some(result) = (unsafe { handle::borrow::<TemplateQueryResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let TemplateTerminalOutcome::Matched(matched) = result.outcome.as_ref() else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    let frame = FrameHandle::new(matched.frame().clone(), result.mapping_observer.clone());
    hooks::reach(hooks::Site::AfterTemporary);
    // SAFETY: out_frame was initialized above; cloning shared ownership neither
    // maps nor copies pixels and retains no strong parent reference.
    unsafe { out_frame.write(handle::into_raw(frame)) };
    MADOPILOT_STATUS_OK
}

pub(crate) fn template_query_result_error(
    result: *const madopilot_template_query_result_t,
    out_failure: *mut *mut madopilot_error_t,
) -> madopilot_status_t {
    // Required terminal data, not optional call-error output.
    // SAFETY: the caller supplies writable owned-error output storage.
    if let Err(fault) = unsafe { boundary::begin_handle_out(out_failure, "out_failure") } {
        return fault.status();
    }
    hooks::reach(hooks::Site::Entry);
    // SAFETY: the caller retains the result for this call.
    let Some(result) = (unsafe { handle::borrow::<TemplateQueryResultHandle>(result) }) else {
        return MADOPILOT_STATUS_INVALID_ARGUMENT;
    };
    if let TemplateTerminalOutcome::Failed(error) = result.outcome.as_ref() {
        let fault = runtime_fault(error);
        hooks::reach(hooks::Site::AfterTemporary);
        // SAFETY: the required output was accepted and initialized above.
        unsafe { out_failure.write(handle::into_raw(fault)) };
    }
    MADOPILOT_STATUS_OK
}

fn pending_snapshot(
    progress: TemplateQueryProgress,
    size: u32,
) -> madopilot_template_query_snapshot_t {
    let mut snapshot = madopilot_template_query_snapshot_t::cleared(size);
    snapshot.state = MADOPILOT_TEMPLATE_QUERY_STATE_PENDING;
    snapshot.query_id = progress.id().get();
    snapshot.generation = progress.generation();
    snapshot.confirmed_observations = progress.confirmed_observations();
    snapshot.confirmed_duration_nanos = nanos(progress.confirmed_duration());
    snapshot.pending_count = progress.pending_count();
    snapshot.in_flight_count = progress.in_flight_count();
    if let Some(frame) = progress.last_frame() {
        snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
        snapshot.last_frame = stamp(frame, struct_size::<madopilot_frame_stamp_t>());
    }
    let work = progress.work();
    snapshot.admitted = work.get(TemplateWorkDisposition::Admitted);
    snapshot.skipped_change = work.get(TemplateWorkDisposition::SkippedChange);
    snapshot.deferred_rate = work.get(TemplateWorkDisposition::DeferredRate);
    snapshot.coalesced = work.get(TemplateWorkDisposition::Coalesced);
    snapshot.superseded = work.get(TemplateWorkDisposition::Superseded);
    snapshot.rejected = work.get(TemplateWorkDisposition::Rejected);
    snapshot.queue_expired = work.get(TemplateWorkDisposition::QueueExpired);
    snapshot.completed = work.get(TemplateWorkDisposition::Completed);
    snapshot.failed = work.get(TemplateWorkDisposition::Failed);
    snapshot
}

fn terminal_info(
    result: &TemplateQueryResultHandle,
    size: u32,
) -> Result<madopilot_template_query_result_info_t, Fault> {
    let mut info = madopilot_template_query_result_info_t::cleared(size);
    info.query_id = result.query_id;
    info.status = result
        .outcome
        .status()
        .map_or(MADOPILOT_STATUS_OK, error::status_code);
    info.outcome = match result.outcome.as_ref() {
        TemplateTerminalOutcome::Matched(matched) => {
            let found = matched.result();
            let effective = found.options();
            info.target = matched.target().get();
            info.source = stamp(
                matched.frame().stamp(),
                struct_size::<madopilot_frame_stamp_t>(),
            );
            info.match_count = u64::try_from(found.matches().len()).unwrap_or(u64::MAX);
            info.template_id = madopilot_str_t::borrowed(matched.template().as_str());
            info.backend_id = madopilot_str_t::borrowed(found.backend().id());
            info.backend_version = madopilot_str_t::borrowed(found.backend().version());
            info.options = madopilot_match_options_t {
                struct_size: struct_size::<madopilot_match_options_t>(),
                flags: MADOPILOT_MATCH_HAS_MIN_SCORE
                    | MADOPILOT_MATCH_HAS_MAX_RESULTS
                    | MADOPILOT_MATCH_HAS_SUPPRESSION,
                min_score: effective.min_score(),
                max_results: effective.max_results(),
                suppression: suppression_code(effective.suppression()),
            };
            info.effective_region = rect(found.searched());
            info.confirmed_observations = matched.confirmed_observations();
            info.confirmed_duration_nanos = nanos(matched.confirmed_duration());
            info.transform = transform_snapshot(matched.frame().transform());
            MADOPILOT_TEMPLATE_QUERY_OUTCOME_MATCHED
        }
        TemplateTerminalOutcome::Cancelled => MADOPILOT_TEMPLATE_QUERY_OUTCOME_CANCELLED,
        TemplateTerminalOutcome::DeadlineExceeded => {
            MADOPILOT_TEMPLATE_QUERY_OUTCOME_DEADLINE_EXCEEDED
        }
        TemplateTerminalOutcome::SessionClosed => MADOPILOT_TEMPLATE_QUERY_OUTCOME_SESSION_CLOSED,
        TemplateTerminalOutcome::SchedulerClosed => {
            MADOPILOT_TEMPLATE_QUERY_OUTCOME_SCHEDULER_CLOSED
        }
        TemplateTerminalOutcome::TargetLost => MADOPILOT_TEMPLATE_QUERY_OUTCOME_TARGET_LOST,
        TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired) => {
            info.overload = MADOPILOT_TEMPLATE_OVERLOAD_QUEUE_EXPIRED;
            MADOPILOT_TEMPLATE_QUERY_OUTCOME_OVERLOADED
        }
        TemplateTerminalOutcome::Failed(_) => MADOPILOT_TEMPLATE_QUERY_OUTCOME_FAILED,
        _ => {
            return Err(Fault::internal(
                "the facade returned an unsupported template terminal outcome",
            ));
        }
    };
    Ok(info)
}

fn transform_snapshot(transform: &TransformSnapshot) -> madopilot_transform_snapshot_t {
    let mut value = madopilot_transform_snapshot_t::cleared(MADOPILOT_TRANSFORM_SNAPSHOT_SIZE_V1_6);
    value.geometry = transform.geometry().value();
    value.width = transform.frame_extent().width();
    value.height = transform.frame_extent().height();
    if transform.covers_target() {
        value.flags |= MADOPILOT_TRANSFORM_COVERS_TARGET;
    }
    if let Some(target) = transform.target() {
        value.flags |= MADOPILOT_TRANSFORM_HAS_TARGET_PLACEMENT;
        (value.desktop_origin_x, value.desktop_origin_y) = target.desktop_origin();
        (value.logical_width, value.logical_height) = target.logical_size();
        value.target_scale_x = target.scale().x();
        value.target_scale_y = target.scale().y();
        value.desktop_scale_x = target.desktop_scale().x();
        value.desktop_scale_y = target.desktop_scale().y();
    }
    value
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn runtime_fault(error: &Error) -> Fault {
    let category = match error.status() {
        Status::Cancelled | Status::DeadlineExceeded => MADOPILOT_ERROR_CATEGORY_OPERATION,
        Status::Closed | Status::TargetLost | Status::CaptureFailed => {
            MADOPILOT_ERROR_CATEGORY_CAPTURE
        }
        Status::VisionFailed => MADOPILOT_ERROR_CATEGORY_VISION,
        Status::AssetInvalid => MADOPILOT_ERROR_CATEGORY_ASSET,
        Status::InputFailed => MADOPILOT_ERROR_CATEGORY_INPUT,
        _ => MADOPILOT_ERROR_CATEGORY_UNSPECIFIED,
    };
    // Runtime Error carries only status/detail; never infer backend or asset data.
    Fault::from_error(error, category)
}
