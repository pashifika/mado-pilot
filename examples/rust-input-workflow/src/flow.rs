//! One engine, one session, one input attempt; submission and observation stay separate.

use std::sync::Arc;
use std::time::Duration;

use mado_pilot::{
    CancellationToken, ChangeDetectionPolicy, CleanupBudget, ClipPolicy, CoordinateSpace,
    DeliveryPlan, Engine, FrameRequest, FrameStamp, InputOpenRequest, InputReceipt, InputRequest,
    InputRequirement, NativeEngineRequest, OcrExecutionProvider, OcrProfile, OcrProfileConfig,
    OcrTextAnalysisRate, OcrTextQueryOutcome, OcrTextStability, OcrTextTerminalOutcome,
    OcrTextWatchRequest, OcrTextWatchResult, OpenRequest, OperationContext, PointerGeometry,
    Session, SessionRequest, Status, TargetKind,
};

use crate::config::{Action, Config, Predicate};
use crate::decisions::{self, ReceiptFacts};
use crate::{Failure, Result};

fn native_engine(
    request: NativeEngineRequest,
    config: &OcrProfileConfig,
    operation: &OperationContext,
) -> Result<Engine> {
    #[cfg(windows)]
    let engine = mado_pilot::windows_engine_with_ocr_profile(request, config, operation);
    #[cfg(target_os = "macos")]
    let engine = mado_pilot::macos_engine_with_ocr_profile(request, config, operation);
    #[cfg(not(any(windows, target_os = "macos")))]
    let engine: mado_pilot::Result<Engine> = {
        let _ = (request, config, operation);
        Err(mado_pilot::Error::new(
            Status::Unsupported,
            "native workflow target unsupported",
        ))
    };
    engine.map_err(|error| Failure::library("engine_construction", error))
}

pub(crate) fn run(config: &Config, allow_input: bool) -> Result<bool> {
    decisions::authorize(&config.action, allow_input)?;
    let cancellation = CancellationToken::new();
    let workflow = OperationContext::new()
        .with_cancellation(cancellation.clone())
        .with_timeout(Duration::from_millis(config.budgets.workflow_ms))
        .map_err(|error| Failure::library("workflow_deadline", error))?;
    let ocr = OcrProfileConfig::new(
        OcrProfile::BoundedDetector,
        config.ocr.model_root.clone(),
        config.ocr.runtime_path.clone(),
    );
    let engine = native_engine(
        NativeEngineRequest::new().with_capture_pacing(config.pacing()?),
        &ocr,
        &workflow,
    )?;
    // No preflight runs before explicit CLI consent. These reads never request permission.
    if engine.reads_permissions() {
        let permissions = engine
            .permissions(&workflow)
            .map_err(|error| Failure::library("permissions", error))?;
        println!(
            "permissions: capture={} input={}",
            permissions.capture().state(),
            permissions.input().state()
        );
        if !permissions.capture().is_granted() {
            return Err(Failure::Policy("screen_capture_permission_not_granted"));
        }
        if config.action.operation().is_some() && !permissions.input().is_granted() {
            return Err(Failure::Policy("input_control_permission_not_granted"));
        }
    } else {
        println!("permissions: probe=unavailable operation_authorization=not_inferred");
    }
    let targets = engine
        .discover(&workflow)
        .map_err(|error| Failure::library("discovery", error))?;
    let target = decisions::unique(targets.iter().filter(|target| {
        target.capability().kind() == Some(TargetKind::Window)
            && target.name() == config.target.window_title
    }))?
    .id();
    drop(targets);
    let mut request = SessionRequest::new().capturing(OpenRequest::new());
    if let Some(kind) = config.action.operation() {
        let input = config
            .input
            .as_ref()
            .ok_or(Failure::Policy("input_policy_required"))?;
        let route = input.route.delivery();
        let descriptor = engine
            .describe_input(target, &workflow)
            .map_err(|error| Failure::library("input_description", error))?;
        let pair = descriptor.capability().pair(kind, route);
        println!(
            "route: operation={kind:?} route={route:?} support={:?} scope={:?} evidence={:?} focus={:?}",
            pair.support(),
            pair.address_scope(),
            pair.evidence(),
            input.focus.policy()
        );
        if !pair.may_attempt() {
            return Err(Failure::Policy("input_pair_unavailable"));
        }
        request = request.requesting_input(
            InputOpenRequest::new()
                .with_requirement(InputRequirement::Required)
                .requiring(kind, route),
        );
    }
    let session = engine
        .open_session(target, &request, &workflow)
        .map_err(|error| Failure::library("session_open", error))?;
    let pacing = session.description().capture_pacing();
    println!(
        "session: target={} stream={} capture_pacing={:?} requested={:?} configured={:?} watcher_admission_ms={}",
        session.target(),
        session.stream(),
        pacing.outcome(),
        pacing.request(),
        pacing.configured_interval(),
        config.analysis_interval_ms
    );

    // Every fallible operation after open is inside this block. Keep the immutable
    // receipt outside it so postcondition, cancellation, and close cannot replace it.
    let mut receipt = None;
    let mut postcondition = None;
    let primary = perform(
        &session,
        config,
        allow_input,
        &workflow,
        &mut receipt,
        &mut postcondition,
    );
    cancellation.cancel();
    // Teardown has independent finite authority, not fresh input authority.
    let close = OperationContext::new()
        .with_clock(workflow.clock())
        .with_timeout(Duration::from_millis(config.budgets.close_ms))
        .map_err(|error| Failure::library("close_deadline", error))
        .and_then(|operation| {
            session
                .close(&operation)
                .map_err(|error| Failure::library("session_close", error))
        });
    drop(session);
    drop(engine);

    if receipt.is_none() {
        println!("input: not_submitted");
    }
    match &postcondition {
        Some(Ok(stamp)) => println!("postcondition: observed source={stamp} causation=not_claimed"),
        Some(Err(error)) => {
            println!("postcondition: not_observed");
            error.report();
        }
        None => println!("postcondition: not_attempted"),
    }
    if let Err(error) = &primary {
        error.report();
    }
    match &close {
        Ok(()) => println!("close: returned physical_backend_quiescence=not_claimed"),
        Err(error) => error.report(),
    }
    Ok(primary.is_ok()
        && close.is_ok()
        && postcondition.as_ref().is_none_or(|result| result.is_ok()))
}

fn perform(
    session: &Session,
    config: &Config,
    allow_input: bool,
    workflow: &OperationContext,
    receipt: &mut Option<InputReceipt>,
    postcondition: &mut Option<Result<FrameStamp>>,
) -> Result<()> {
    let terminal = watch(session, &config.before, config, workflow, "before")?;
    let observation = accepted(&terminal, session)?;
    println!(
        "observation: matched source={} satisfying_regions={}",
        observation.result().stamp(),
        observation.satisfying_region_indexes().len()
    );
    if !decisions::may_send(&config.action, allow_input, true, workflow)? {
        println!("mode: observe_only");
        return Ok(());
    }
    let source = observation.result().stamp();
    let point = if matches!(config.action, Action::Click) {
        Some(decisions::click_point(observation)?)
    } else {
        None
    };
    let sequence = decisions::recipe(&config.action, point)?;
    let input = config
        .input
        .as_ref()
        .ok_or(Failure::Policy("input_policy_required"))?;
    let request = InputRequest::new(
        session.target(),
        sequence,
        DeliveryPlan::require(input.route.delivery()),
    )
    .with_focus(input.focus.policy())
    .with_pointer_geometry(PointerGeometry::require_unchanged_since(source))
    .with_cleanup_budget(CleanupBudget::at_most(10, Duration::from_millis(250)));
    session
        .input_descriptor()
        .validate(&request)
        .map_err(|fault| Failure::Input {
            stage: "input_validation",
            fault,
        })?;
    session
        .input_descriptor()
        .preflight_route(&request, input.route.delivery())
        .map_err(|fault| Failure::Input {
            stage: "input_route_preflight",
            fault,
        })?;

    // This is the current publication immediately before submission, not the older
    // matching OCR stamp. Content can still change after this check (visual TOCTOU).
    let current = session
        .acquire_frame(&FrameRequest::latest(), workflow)
        .map_err(|error| Failure::library("pre_input_checkpoint", error))?;
    let checkpoint = current.stamp();
    decisions::require_action_geometry(source, checkpoint)?;
    drop(current);
    println!(
        "pre_input: checkpoint={checkpoint} geometry_guard_source={source} visual_atomicity=not_claimed"
    );
    decisions::may_send(&config.action, allow_input, true, workflow)?;
    let submitted = session
        .send_input(&request, workflow)
        .map_err(|error| Failure::library("input_submission", error))?;
    let facts = ReceiptFacts::from(&submitted);
    *receipt = Some(submitted);
    if let Some(receipt) = receipt.as_ref() {
        report_receipt(receipt);
    }
    drop(terminal);
    if facts.should_observe() {
        // A partial receipt remains primary even if independent observation fails.
        *postcondition = Some(postcondition_after(session, config, checkpoint, workflow));
    }
    if !facts.submission_complete() {
        return Err(Failure::Policy(
            "input_not_completely_submitted_or_cleanup_incomplete",
        ));
    }
    Ok(())
}

fn watch(
    session: &Session,
    predicate: &Predicate,
    config: &Config,
    authority: &OperationContext,
    stage: &'static str,
) -> Result<Arc<OcrTextTerminalOutcome>> {
    let request = OcrTextWatchRequest::new(
        predicate.roi.rect()?,
        ClipPolicy::Reject,
        &predicate.literal,
        predicate.minimum_confidence,
        CoordinateSpace::CapturePixels,
        OcrTextAnalysisRate::from_minimum_interval(Duration::from_millis(
            config.analysis_interval_ms,
        ))
        .map_err(|error| Failure::library("watcher_admission_rate", error))?,
        OcrTextStability::immediate(),
        ChangeDetectionPolicy::ExactRgba,
        authority.clone(),
    )
    .map_err(|error| Failure::library(stage, error))?;
    // Prepare the independent wait before admitting the query: setup failure then
    // cannot leave an unowned pending query. Neither context extends the parent.
    let wait = decisions::bounded_child(authority, Duration::from_millis(config.budgets.wait_ms))?;
    let query = session
        .start_ocr_text_watch(request)
        .map_err(|error| Failure::library(stage, error))?;
    let state = match query.poll() {
        OcrTextQueryOutcome::Pending(_) => "pending",
        OcrTextQueryOutcome::Terminal(_) => "terminal",
    };
    println!("query: stage={stage} polled={state}");
    let waited = query.wait(&wait);
    if waited.is_err() {
        let _winner = query.cancel();
    }
    drop(query); // Cancels pending work, not a physical-inference interruption guarantee.
    let terminal = waited.map_err(|error| Failure::library(stage, error))?;
    println!(
        "query: stage={stage} matched={} status={:?}",
        terminal.is_match(),
        terminal.status()
    );
    Ok(terminal)
}

fn accepted<'a>(
    terminal: &'a OcrTextTerminalOutcome,
    session: &Session,
) -> Result<&'a OcrTextWatchResult> {
    let OcrTextTerminalOutcome::Matched(result) = terminal else {
        return match terminal {
            OcrTextTerminalOutcome::Failed(error) => {
                Err(Failure::library("ocr_terminal", error.clone()))
            }
            _ => Err(Failure::library(
                "ocr_terminal",
                mado_pilot::Error::new(
                    terminal.status().unwrap_or(Status::Unsupported),
                    "OCR did not match",
                ),
            )),
        };
    };
    if result.target() != session.target()
        || result.result().stamp().stream() != session.stream()
        || result.result().stamp() != result.frame().stamp()
        || result.result().transform() != result.frame().transform()
        || result.result().output_space() != CoordinateSpace::CapturePixels
        || result.provider().active_provider() != OcrExecutionProvider::Cpu
    {
        return Err(Failure::Policy("ocr_result_correlation"));
    }
    Ok(result)
}

fn postcondition_after(
    session: &Session,
    config: &Config,
    checkpoint: FrameStamp,
    workflow: &OperationContext,
) -> Result<FrameStamp> {
    let authority = decisions::bounded_child(
        workflow,
        Duration::from_millis(config.budgets.postcondition_ms),
    )?;
    let predicate = config
        .after
        .as_ref()
        .ok_or(Failure::Policy("postcondition_required"))?;
    let fresh = session
        .acquire_frame(&FrameRequest::newer_than(checkpoint), &authority)
        .map_err(|error| Failure::library("postcondition_newer_frame", error))?;
    if !decisions::newer(checkpoint, fresh.stamp())? {
        return Err(Failure::Policy("newer_frame_request_did_not_advance"));
    }
    drop(fresh);
    // Acquisition is not a watcher publication fence. Re-arm stale matches
    // without skipping the already-available newer frame on a static screen.
    for _ in 0..32 {
        let terminal = watch(session, predicate, config, &authority, "postcondition")?;
        let result = accepted(&terminal, session)?;
        let source = result.result().stamp();
        if decisions::postcondition_progress(
            checkpoint.is_same_stream(&source),
            checkpoint.into(),
            source.into(),
        )? {
            decisions::checkpoint(&authority)?;
            return Ok(source);
        }
        println!("postcondition: ignored_stale_match");
    }
    Err(Failure::Policy("postcondition_observation_limit"))
}

fn report_receipt(receipt: &InputReceipt) {
    println!(
        "input: target={} outcome={:?} route={:?} scope={:?} submitted={} last_submitted={:?} evidence={:?} possible_effect={} partial_native_effect={} fault={:?} cleanup={:?} cleanup_released={} cleanup_owed={}",
        receipt.target(),
        receipt.outcome(),
        receipt.selected_route(),
        receipt.address_scope(),
        receipt.submitted(),
        receipt.last_submitted(),
        receipt.evidence(),
        receipt.possible_native_effect(),
        receipt.partial_native_effect(),
        receipt.fault(),
        receipt.cleanup(),
        receipt.cleanup_released(),
        receipt.cleanup_owed()
    );
    for (index, attempt) in receipt.attempts().iter().enumerate() {
        println!(
            "input_attempt: index={index} route={:?} scope={:?} outcome={:?} submitted={} last_submitted={:?} evidence={:?} possible_effect={} partial_native_effect={} fault={:?}",
            attempt.route(),
            attempt.address_scope(),
            attempt.outcome(),
            attempt.submitted(),
            attempt.last_submitted(),
            attempt.evidence(),
            attempt.possible_native_effect(),
            attempt.partial_native_effect(),
            attempt.fault()
        );
    }
}
