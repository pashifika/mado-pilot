//! Checked WGC interval conversion and pre-start capability negotiation.

use std::mem;
use std::time::Duration;

use mado_pilot_capture::{CapturePacingReport, PacingUnsupportedReason, ResolvedCapturePacing};
use mado_pilot_core::{Error, Operation, Result, Status, TargetKind};
use windows::Foundation::Metadata::{ApiInformation, IApiInformationStatics};
use windows::Foundation::TimeSpan;
use windows::Graphics::Capture::{GraphicsCaptureSession, IGraphicsCaptureSession5};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOINTERFACE};
use windows::core::{HSTRING, Interface, RuntimeName};

use crate::native::native_target_fault;
use crate::optional_api::activation_factory;

const NANOS_PER_TICK: u128 = 100;
const TICKS_PER_SECOND: u64 = 10_000_000;

pub(crate) fn interval_ticks(interval: Duration) -> Result<i64> {
    if interval.is_zero() {
        return Err(Error::new(
            Status::InvalidArgument,
            "Windows capture pacing interval must be positive",
        ));
    }
    interval
        .as_nanos()
        .checked_add(NANOS_PER_TICK - 1)
        .and_then(|nanos| i64::try_from(nanos / NANOS_PER_TICK).ok())
        .ok_or_else(|| {
            Error::new(
                Status::InvalidArgument,
                "Windows capture pacing interval exceeds signed 64-bit 100ns ticks",
            )
        })
}

pub(crate) fn configure_capture_pacing(
    capture: &GraphicsCaptureSession,
    request: ResolvedCapturePacing,
    kind: TargetKind,
    operation: &mut Operation<'_>,
) -> Result<CapturePacingReport> {
    negotiate_with(
        request,
        kind,
        operation,
        || capture.cast::<IGraphicsCaptureSession5>(),
        pacing_property_present,
        set_interval,
        read_interval,
    )
}

fn pacing_property_present() -> windows::core::Result<bool> {
    // Use the existing controlled loader, not the generated static factory cache.
    let factory: IApiInformationStatics = activation_factory(ApiInformation::NAME)?;
    let class = HSTRING::from(GraphicsCaptureSession::NAME);
    let property = HSTRING::from("MinUpdateInterval");
    let mut present = false;
    // SAFETY: factory is the documented metadata interface, both HSTRINGs stay
    // live for the call, and present is writable. HSTRING has pointer ABI layout.
    unsafe {
        (Interface::vtable(&factory).IsWriteablePropertyPresent)(
            Interface::as_raw(&factory),
            mem::transmute_copy(&class),
            mem::transmute_copy(&property),
            &raw mut present,
        )
    }
    .ok()?;
    Ok(present)
}

fn set_interval(control: &IGraphicsCaptureSession5, ticks: i64) -> windows::core::Result<()> {
    // SAFETY: QueryInterface established this exact interface and ticks is a
    // checked positive TimeSpan value. No callback has been registered yet.
    unsafe {
        (Interface::vtable(control).SetMinUpdateInterval)(
            Interface::as_raw(control),
            TimeSpan { Duration: ticks },
        )
    }
    .ok()
}

fn read_interval(control: &IGraphicsCaptureSession5) -> windows::core::Result<i64> {
    let mut interval = TimeSpan { Duration: 0 };
    // SAFETY: control is a live negotiated interface and interval is writable
    // for the complete TimeSpan output. Zero cannot pass our postcondition.
    unsafe {
        (Interface::vtable(control).MinUpdateInterval)(
            Interface::as_raw(control),
            &raw mut interval,
        )
    }
    .ok()?;
    Ok(interval.Duration)
}

fn negotiate_with<C>(
    request: ResolvedCapturePacing,
    kind: TargetKind,
    operation: &mut Operation<'_>,
    query_interface: impl FnOnce() -> windows::core::Result<C>,
    property_present: impl FnOnce() -> windows::core::Result<bool>,
    set: impl FnOnce(&C, i64) -> windows::core::Result<()>,
    get: impl FnOnce(&C) -> windows::core::Result<i64>,
) -> Result<CapturePacingReport> {
    let Some(interval) = request.interval() else {
        return Ok(CapturePacingReport::source_default());
    };
    let requested_ticks = interval_ticks(interval)?;
    operation.checkpoint()?;
    let control = query_interface();
    operation.checkpoint()?;
    let control = match control {
        Ok(control) => control,
        Err(error) if error.code() == E_NOINTERFACE => return unavailable(request),
        Err(error) => return Err(configuration_error(error, kind)),
    };
    let present = property_present();
    operation.checkpoint()?;
    if !present.map_err(|error| configuration_error(error, kind))? {
        return unavailable(request);
    }

    // Once the control is present, even E_NOINTERFACE/E_NOTIMPL from a property
    // call is a configuration failure, never a preferred capability fallback.
    let configured = set(&control, requested_ticks);
    operation.checkpoint()?;
    configured.map_err(|error| configuration_error(error, kind))?;
    let configured = get(&control);
    operation.checkpoint()?;
    let configured = configured.map_err(|error| configuration_error(error, kind))?;
    if configured < requested_ticks {
        return Err(Error::new(
            Status::CaptureFailed,
            "Windows capture pacing readback did not establish the requested minimum",
        ));
    }
    let ticks = u64::try_from(configured).expect("readback is at least the positive request");
    let configured = Duration::new(
        ticks / TICKS_PER_SECOND,
        u32::try_from((ticks % TICKS_PER_SECOND) * 100).expect("fraction is below one second"),
    );
    CapturePacingReport::applied(request, configured)
}

fn unavailable(request: ResolvedCapturePacing) -> Result<CapturePacingReport> {
    CapturePacingReport::unsupported(request, PacingUnsupportedReason::NativeControlUnavailable)
}

fn configuration_error(error: windows::core::Error, kind: TargetKind) -> Error {
    if error.code() == E_INVALIDARG {
        Error::new(
            Status::InvalidArgument,
            "Windows capture pacing control rejected the interval",
        )
    } else {
        native_target_fault(error, kind).into()
    }
}

#[cfg(test)]
mod tests {
    // These controlled seams do not qualify native WGC behavior on the test host.
    use std::cell::Cell;
    use std::sync::Arc;
    use std::time::Duration;

    use mado_pilot_capture::{
        CapturePacingOutcome, CapturePacingRequest, PacingUnsupportedReason, ResolvedCapturePacing,
    };
    use mado_pilot_core::{
        CancellationToken, MonotonicInstant, Operation, OperationContext, Status, TargetKind,
    };
    use mado_pilot_testkit::ManualClock;
    use windows::Win32::Foundation::{
        E_ACCESSDENIED, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, RO_E_CLOSED,
    };
    use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
    use windows::core::Error;

    use super::{interval_ticks, negotiate_with};

    fn resolved(interval: Duration, required: bool) -> ResolvedCapturePacing {
        let request = if required {
            CapturePacingRequest::required(interval)
        } else {
            CapturePacingRequest::preferred(interval)
        };
        request
            .expect("positive request")
            .resolve(ResolvedCapturePacing::source_default())
    }

    #[test]
    fn positive_intervals_round_up_across_tick_and_second_boundaries() {
        for (interval, ticks) in [
            (Duration::from_nanos(1), 1),
            (Duration::from_nanos(99), 1),
            (Duration::from_nanos(100), 1),
            (Duration::from_nanos(101), 2),
            (Duration::new(1, 1), 10_000_001),
        ] {
            assert_eq!(interval_ticks(interval).expect("representable"), ticks);
        }
    }

    #[test]
    fn signed_tick_limit_accepts_its_last_value_and_rejects_rounding_overflow() {
        let maximum = Duration::new(922_337_203_685, 477_580_700);
        assert_eq!(interval_ticks(maximum).expect("maximum ticks"), i64::MAX);
        for invalid in [
            Duration::ZERO,
            maximum + Duration::from_nanos(1),
            Duration::MAX,
        ] {
            assert_eq!(
                interval_ticks(invalid)
                    .expect_err("unrepresentable")
                    .status(),
                Status::InvalidArgument
            );
        }
    }

    #[test]
    fn source_default_never_probes_an_unavailable_host_control() {
        let context = OperationContext::new();
        let report = negotiate_with::<()>(
            ResolvedCapturePacing::source_default(),
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || panic!("source default must not query the optional interface"),
            || panic!("source default must not query metadata"),
            |_, _| panic!("source default must not configure cadence"),
            |_| panic!("source default must not read native cadence"),
        )
        .expect("source default needs no optional native control");
        assert_eq!(report.outcome(), CapturePacingOutcome::SourceDefault);
        assert_eq!(report.configured_interval(), None);
    }

    #[test]
    fn unrepresentable_preference_fails_before_runtime_negotiation() {
        let context = OperationContext::new();
        let error = negotiate_with::<()>(
            resolved(Duration::MAX, false),
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || panic!("invalid selected duration must fail before native work"),
            || panic!("invalid selected duration must fail before metadata"),
            |_, _| panic!("invalid interval cannot reach a setter"),
            |_| panic!("invalid interval cannot reach a getter"),
        )
        .expect_err("preference does not excuse representation overflow");
        assert_eq!(error.status(), Status::InvalidArgument);
    }

    #[test]
    fn missing_interface_and_missing_property_are_the_only_unapplied_preferences() {
        let context = OperationContext::new();
        for interface_missing in [true, false] {
            for required in [true, false] {
                let request = resolved(Duration::from_millis(30), required);
                let result = negotiate_with(
                    request,
                    TargetKind::Window,
                    &mut Operation::admit(&context).expect("admitted"),
                    || {
                        if interface_missing {
                            Err(Error::from_hresult(E_NOINTERFACE))
                        } else {
                            Ok(())
                        }
                    },
                    || {
                        assert!(!interface_missing, "missing interface ends negotiation");
                        Ok(false)
                    },
                    |_, _| panic!("absent control cannot be configured"),
                    |_| panic!("absent control cannot be read"),
                );
                if required {
                    assert_eq!(
                        result.expect_err("required control missing").status(),
                        Status::Unsupported
                    );
                } else {
                    let report = result.expect("explicit unapplied preference");
                    assert_eq!(report.request(), request);
                    assert_eq!(report.configured_interval(), None);
                    assert_eq!(
                        report.outcome(),
                        CapturePacingOutcome::PreferredUnapplied(
                            PacingUnsupportedReason::NativeControlUnavailable
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn failed_capability_queries_are_not_proof_of_absence() {
        let context = OperationContext::new();
        let request = resolved(Duration::from_millis(30), false);
        let query_error = negotiate_with::<()>(
            request,
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || Err(Error::from_hresult(E_NOTIMPL)),
            || panic!("query failure is terminal"),
            |_, _| panic!("query failure is terminal"),
            |_| panic!("query failure is terminal"),
        )
        .expect_err("only QueryInterface E_NOINTERFACE proves interface absence");
        assert_eq!(query_error.status(), Status::CaptureFailed);

        let metadata_error = negotiate_with(
            request,
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || Ok(()),
            || Err(Error::from_hresult(E_NOINTERFACE)),
            |_, _| panic!("metadata failure is terminal"),
            |_| panic!("metadata failure is terminal"),
        )
        .expect_err("failed metadata lookup does not prove the property is absent");
        assert_eq!(metadata_error.status(), Status::CaptureFailed);
    }

    #[test]
    fn available_setter_errors_preserve_failure_even_for_preferences() {
        let context = OperationContext::new();
        for (code, status) in [
            (E_ACCESSDENIED, Status::CaptureFailed),
            (E_INVALIDARG, Status::InvalidArgument),
            (DXGI_ERROR_DEVICE_REMOVED, Status::CaptureFailed),
            (DXGI_ERROR_DEVICE_RESET, Status::CaptureFailed),
            (RO_E_CLOSED, Status::TargetLost),
            (E_NOINTERFACE, Status::CaptureFailed),
            (E_NOTIMPL, Status::CaptureFailed),
        ] {
            let error = negotiate_with(
                resolved(Duration::from_millis(30), false),
                TargetKind::Window,
                &mut Operation::admit(&context).expect("admitted"),
                || Ok(()),
                || Ok(true),
                |_, _| Err(Error::from_hresult(code)),
                |_| panic!("a rejected setter cannot establish an interval"),
            )
            .expect_err("available control failed");
            assert_eq!(error.status(), status);
        }
    }

    #[test]
    fn getter_failure_or_shorter_unestablished_readback_never_reports_applied() {
        let context = OperationContext::new();
        for readback in [Err(E_NOINTERFACE), Err(RO_E_CLOSED), Ok(-1), Ok(0), Ok(1)] {
            let error = negotiate_with(
                resolved(Duration::from_nanos(101), false),
                TargetKind::Display,
                &mut Operation::admit(&context).expect("admitted"),
                || Ok(()),
                || Ok(true),
                |_, _| Ok(()),
                |_| readback.map_err(Error::from_hresult),
            )
            .expect_err("successful setter is insufficient without valid readback");
            let expected = if readback == Err(RO_E_CLOSED) {
                Status::TargetLost
            } else {
                Status::CaptureFailed
            };
            assert_eq!(error.status(), expected);
        }
    }

    #[test]
    fn applied_report_uses_native_readback_without_shortening_or_nanos_overflow() {
        let context = OperationContext::new();
        for (ticks, interval) in [
            (2, Duration::from_nanos(200)),
            (3, Duration::from_nanos(300)),
            (i64::MAX, Duration::new(922_337_203_685, 477_580_700)),
        ] {
            let selected = resolved(Duration::from_nanos(101), true);
            let written = Cell::new(None);
            let report = negotiate_with(
                selected,
                TargetKind::Window,
                &mut Operation::admit(&context).expect("admitted"),
                || Ok(()),
                || Ok(true),
                |_, value| {
                    written.set(Some(value));
                    Ok(())
                },
                |_| {
                    assert_eq!(
                        written.get(),
                        Some(2),
                        "readback follows the rounded assignment"
                    );
                    Ok(ticks)
                },
            )
            .expect("established configuration");
            assert_eq!(report.outcome(), CapturePacingOutcome::Applied);
            assert_eq!(report.request(), selected);
            assert_eq!(report.configured_interval(), Some(interval));
        }
    }

    #[test]
    fn cancellation_during_assignment_wins_without_readback_or_a_report() {
        let token = CancellationToken::new();
        let context = OperationContext::new().with_cancellation(token.clone());
        let error = negotiate_with(
            resolved(Duration::from_millis(30), false),
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || Ok(()),
            || Ok(true),
            |_, _| {
                token.cancel();
                Err(Error::from_hresult(E_ACCESSDENIED))
            },
            |_| panic!("cancelled assignment cannot continue to readback"),
        )
        .expect_err("cancellation wins after the native call");
        assert_eq!(error.status(), Status::Cancelled);
    }

    #[test]
    fn expired_authority_rejects_an_otherwise_valid_native_readback() {
        let clock = Arc::new(ManualClock::new());
        let context = OperationContext::new()
            .with_clock(clock.clone())
            .with_deadline(MonotonicInstant::from_origin(Duration::from_secs(1)));
        let error = negotiate_with(
            resolved(Duration::from_millis(30), true),
            TargetKind::Window,
            &mut Operation::admit(&context).expect("admitted"),
            || Ok(()),
            || Ok(true),
            |_, _| Ok(()),
            |_| {
                clock.advance(Duration::from_secs(1));
                Ok(300_000)
            },
        )
        .expect_err("a late configured result cannot become a report");
        assert_eq!(error.status(), Status::DeadlineExceeded);
    }
}
