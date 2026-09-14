//! One exact OCR admission per authenticated observation, with caller-owned cooldown.

use std::time::Duration;

use mado_pilot::{
    ClipPolicy, CoordinateSpace, CpuMapping, Frame, FrameOrder, FrameRequest, FrameStamp,
    MonotonicInstant, OcrBackendDescriptor, OcrRegion, OperationContext, PixelExtent, PixelFormat,
    PixelRect, Rect, Session,
};

use super::completion_cooldown::{checkpoint, cooldown, recognize_exact};
use super::contract::{
    Check, Failure, MAPPING_LIMIT, Report, SAMPLE_LIMIT, Sample, api, nanos, require,
};
use super::control::{Ack, Fixture, Marker, marker};

const LITERAL: &str = "魔導士";

#[derive(Default)]
pub(super) struct Cursor {
    pub(super) last: Option<FrameStamp>,
    pub(super) completed_at: Option<MonotonicInstant>,
}

pub(super) struct Observation {
    pub(super) frame: Frame,
    pub(super) marker: Marker,
    pub(super) acquired_at: MonotonicInstant,
}

#[derive(Clone, Copy)]
pub(super) struct Acquisition {
    pub(super) after: Option<FrameStamp>,
    pub(super) expected: Ack,
    pub(super) exact_counter: bool,
    pub(super) retained_bytes: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Recognition {
    pub(super) expected_state: u32,
    pub(super) required_cooldown: Duration,
}

pub(super) struct KnownPublication {
    stamp: FrameStamp,
    marker: Marker,
    extent: PixelExtent,
}

impl Observation {
    pub(super) fn publication(&self) -> KnownPublication {
        KnownPublication {
            stamp: self.frame.stamp(),
            marker: self.marker,
            extent: self.frame.descriptor().extent(),
        }
    }
}

impl KnownPublication {
    fn verify_first(self, acquire: impl FnOnce() -> Check<Observation>) -> Check<Observation> {
        let first = acquire()?;
        require(
            matches!(
                first.frame.stamp().order(&self.stamp),
                Ok(FrameOrder::Same | FrameOrder::After)
            ) && first.marker == self.marker
                && first.frame.descriptor().extent() == self.extent,
            "first-acquisition-before-known-publication",
        )?;
        Ok(first)
    }
}

fn add(value: &mut u64, amount: u64) -> Check<()> {
    *value = value
        .checked_add(amount)
        .ok_or(Failure::Rule("metric-counter-overflow"))?;
    Ok(())
}

pub(super) fn layout(frame: &Frame, retained_bytes: u64, report: &mut Report) -> Check<()> {
    let bytes = frame.descriptor().byte_len() as u64;
    require(bytes <= MAPPING_LIMIT, "frame-layout-budget")?;
    let total = retained_bytes
        .checked_add(bytes)
        .ok_or(Failure::Rule("retained-layout-budget"))?;
    let limit = if report.case == "semantic" {
        3 * MAPPING_LIMIT
    } else {
        MAPPING_LIMIT
    };
    require(total <= limit, "retained-layout-budget")?;
    report.metrics.max_retained_layout_bytes = report.metrics.max_retained_layout_bytes.max(total);
    Ok(())
}

pub(super) fn map(
    frame: &Frame,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<CpuMapping> {
    api(checkpoint(operation))?;
    require(
        frame.descriptor().byte_len() as u64 <= MAPPING_LIMIT,
        "frame-layout-budget",
    )?;
    let mapping = api(frame.map(PixelFormat::Bgra8, operation))?;
    let bytes = mapping.descriptor().byte_len() as u64;
    add(&mut report.metrics.caller_mapped_bytes, bytes)?;
    report.metrics.max_mapping_bytes = report.metrics.max_mapping_bytes.max(bytes);
    require(bytes <= MAPPING_LIMIT, "mapping-budget")?;
    require(
        mapping.stamp() == frame.stamp()
            && mapping.transform() == frame.transform()
            && mapping.descriptor().extent() == frame.descriptor().extent()
            && mapping.bytes().len() == mapping.descriptor().byte_len(),
        "mapping-identity",
    )?;
    Ok(mapping)
}

fn acquire_once(
    session: &Session,
    fixture: &mut Fixture,
    after: Option<FrameStamp>,
    retained_bytes: u64,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Observation> {
    api(checkpoint(operation))?;
    fixture.healthy()?;
    let request = after.map_or_else(FrameRequest::latest, FrameRequest::newer_than);
    let frame = api(session.acquire_frame(&request, operation))?;
    let acquired_at = operation.now();
    require(
        frame.stamp().stream() == session.stream(),
        "frame-stream-identity",
    )?;
    if let Some(previous) = after {
        require(
            frame.stamp().order(&previous) == Ok(FrameOrder::After),
            "frame-order",
        )?;
    }
    layout(&frame, retained_bytes, report)?;
    require(
        [PixelExtent::new(960, 576), PixelExtent::new(1040, 640)]
            .contains(&frame.descriptor().extent())
            && frame.transform().frame_extent() == frame.descriptor().extent()
            && frame.transform().geometry() == frame.stamp().geometry(),
        "frame-geometry",
    )?;
    let observed = {
        let mapping = map(&frame, operation, report)?;
        marker(mapping.descriptor(), mapping.bytes(), fixture.token)?
    }; // The oracle mapping never crosses OCR admission or cooldown.
    report.check("owned_source", true)?;
    Ok(Observation {
        frame,
        marker: observed,
        acquired_at,
    })
}

// Authentication polling only: this must not decide a latest-after-cooldown claim.
pub(super) fn acquire(
    session: &Session,
    fixture: &mut Fixture,
    request: Acquisition,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Observation> {
    let mut last = request.after;
    loop {
        let observation = acquire_once(
            session,
            fixture,
            last,
            request.retained_bytes,
            operation,
            report,
        )?;
        if observation.marker.counter >= request.expected.counter
            && observation.marker.state == request.expected.state
            && observation.frame.descriptor().extent() == request.expected.extent
        {
            require(
                !request.exact_counter || observation.marker.counter == request.expected.counter,
                "fixture-counter-ahead",
            )?;
            return Ok(observation);
        }
        last = Some(observation.frame.stamp());
    }
}

pub(super) fn acquire_known(
    session: &Session,
    fixture: &mut Fixture,
    request: Acquisition,
    known: KnownPublication,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<Observation> {
    // Exactly one acquisition: a stale first result fails, even if a later one would match.
    known.verify_first(|| {
        acquire_once(
            session,
            fixture,
            request.after,
            request.retained_bytes,
            operation,
            report,
        )
    })
}

pub(super) fn age(now: MonotonicInstant, captured_at: MonotonicInstant) -> Check<Option<u64>> {
    if captured_at > now {
        Ok(None)
    } else {
        nanos(now.saturating_duration_since(captured_at)).map(Some)
    }
}

fn admit(operation: &OperationContext, report: &mut Report) -> Check<()> {
    api(checkpoint(operation))?;
    report.check("sample_capacity", report.samples.len() < SAMPLE_LIMIT)?;
    add(&mut report.metrics.ocr_admissions, 1)
}

pub(super) fn sample_capacity(report: &mut Report) -> Check<()> {
    report.check("sample_capacity", report.samples.len() <= SAMPLE_LIMIT)
}

pub(super) fn recognize(
    session: &Session,
    backend: &OcrBackendDescriptor,
    observation: Observation,
    cursor: &mut Cursor,
    policy: Recognition,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<FrameStamp> {
    api(checkpoint(operation))?;
    let stamp = observation.frame.stamp();
    require(
        observation.marker.state == policy.expected_state,
        "ocr-source-state",
    )?;
    let gap = cursor
        .completed_at
        .map(|completed| nanos(observation.acquired_at.saturating_duration_since(completed)))
        .transpose()?;
    report.check(
        "cooldown_spacing",
        gap.is_none_or(|gap| u128::from(gap) >= policy.required_cooldown.as_nanos()),
    )?;
    if let Some(previous) = cursor.last {
        report.check(
            "source_identity",
            stamp.order(&previous) == Ok(FrameOrder::After),
        )?;
    }
    let frame_age_ns = age(observation.acquired_at, observation.frame.captured_at())?;
    let region = OcrRegion::Region {
        rect: Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 960.0, 540.0)
            .map_err(|_| Failure::Rule("ocr-region"))?,
        policy: ClipPolicy::Reject,
    };
    admit(operation, report)?;
    let started = operation.now();
    let result = recognize_exact(session, backend, &observation.frame, region, operation);
    let ocr_ns = nanos(operation.now().saturating_duration_since(started))?;
    report.samples.push(Sample {
        ocr_ns,
        frame_age_ns,
        cooldown_gap_ns: gap,
        stream_id: stamp.stream().get(),
        sequence: stamp.sequence().value(),
        epoch: stamp.epoch().value(),
        geometry_revision: stamp.geometry().value(),
    });
    let result = api(result)?;
    api(checkpoint(operation))?;
    let expected_region =
        PixelRect::new(0, 0, 960, 540).map_err(|_| Failure::Rule("ocr-region"))?;
    report.check(
        "source_identity",
        result.stamp() == stamp
            && result.transform() == observation.frame.transform()
            && result.effective_region() == expected_region
            && result.output_space() == CoordinateSpace::CapturePixels
            && result.backend() == backend,
    )?;
    let mut found = false;
    for region in result.regions() {
        api(checkpoint(operation))?;
        if region.text() == LITERAL {
            found = true;
            break;
        }
    }
    require(
        if policy.expected_state == 0 {
            result.is_empty()
        } else {
            found
        },
        "ocr-literal",
    )?;
    api(checkpoint(operation))?;
    cursor.completed_at = Some(operation.now());
    cursor.last = Some(stamp);
    add(&mut report.metrics.ocr_committed, 1)?;
    drop(result);
    drop(observation);
    Ok(stamp)
}

pub(super) fn step(
    session: &Session,
    backend: &OcrBackendDescriptor,
    fixture: &mut Fixture,
    cursor: &mut Cursor,
    interval: Duration,
    operation: &OperationContext,
    report: &mut Report,
) -> Check<()> {
    api(checkpoint(operation))?;
    let expected = fixture.ack;
    let observation = acquire(
        session,
        fixture,
        Acquisition {
            after: cursor.last,
            expected,
            exact_counter: false,
            retained_bytes: 0,
        },
        operation,
        report,
    )?;
    recognize(
        session,
        backend,
        observation,
        cursor,
        Recognition {
            expected_state: 1,
            required_cooldown: interval,
        },
        operation,
        report,
    )?;
    // All observation/result/mapping owners are released before this full delay.
    api(cooldown(interval, operation, &mut std::thread::sleep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mado_pilot::{CancellationToken, Continuity, FrameDescriptor, Status};
    use mado_pilot_runtime::IdentityIssuer;
    use mado_pilot_testkit::{Publication, StreamState};

    fn published(state: &StreamState, counter: u32) -> Observation {
        let descriptor = FrameDescriptor::packed(PixelExtent::new(32, 24), PixelFormat::Bgra8)
            .expect("controlled descriptor");
        let frame = state
            .publish(Publication {
                captured_at: MonotonicInstant::ORIGIN,
                descriptor,
                placement: None,
                continuity: Continuity::Continuous,
                pixels: vec![0; descriptor.byte_len()].into_boxed_slice(),
            })
            .expect("controlled publication");
        Observation {
            frame,
            marker: Marker { counter, state: 1 },
            acquired_at: MonotonicInstant::ORIGIN,
        }
    }

    #[test]
    fn stale_first_acquisition_fails_without_draining_to_the_known_publication() {
        let state = StreamState::new(IdentityIssuer::new().issue_stream().expect("stream"));
        let consumed = published(&state, 8).frame.stamp();
        let stale = published(&state, 9);
        assert_eq!(stale.frame.stamp().order(&consumed), Ok(FrameOrder::After));
        let final_publication = published(&state, 9);
        let final_stamp = final_publication.frame.stamp();
        let known = final_publication.publication();
        let mut queued = [stale, final_publication].into_iter();
        let mut acquisitions = 0;
        let result = known.verify_first(|| {
            acquisitions += 1;
            queued
                .next()
                .ok_or(Failure::Rule("controlled-source-empty"))
        });
        assert!(result.is_err());
        assert_eq!(acquisitions, 1);
        assert_eq!(
            queued
                .next()
                .expect("later publication was not drained")
                .frame
                .stamp(),
            final_stamp
        );
    }

    #[test]
    fn known_publication_accepts_same_or_later_stamp_but_not_changed_marker() {
        let state = StreamState::new(IdentityIssuer::new().issue_stream().expect("stream"));
        let original = published(&state, 9);
        let same = original
            .publication()
            .verify_first(|| Ok(original))
            .expect("same publication");
        let later = published(&state, 9);
        let later = same
            .publication()
            .verify_first(|| Ok(later))
            .expect("same pixels with later stamp");
        let changed = published(&state, 10);
        assert!(later.publication().verify_first(|| Ok(changed)).is_err());
    }

    #[test]
    fn interrupted_admission_does_not_change_counts_or_publish_samples() {
        for cancelled in [false, true] {
            let mut report = Report::new();
            let operation = OperationContext::new();
            let operation = if cancelled {
                let token = CancellationToken::new();
                token.cancel();
                operation.with_cancellation(token)
            } else {
                operation.clone().with_deadline(operation.now())
            };
            let failure = admit(&operation, &mut report).expect_err("interrupted admission");
            assert_eq!(
                failure,
                Failure::Api(if cancelled {
                    Status::Cancelled
                } else {
                    Status::DeadlineExceeded
                })
            );
            assert_eq!(report.metrics.ocr_admissions, 0);
            assert!(report.samples.is_empty());
        }
    }

    #[test]
    fn future_capture_timestamp_is_unavailable_not_zero_age() {
        let now = MonotonicInstant::from_origin(Duration::from_secs(1));
        assert_eq!(age(now, now).expect("same clock"), Some(0));
        assert_eq!(
            age(
                now,
                now.checked_add(Duration::from_nanos(1)).expect("future")
            )
            .expect("age"),
            None
        );
    }

    #[test]
    fn sample_capacity_refuses_the_next_ocr_admission() {
        let mut report = Report::new();
        sample_capacity(&mut report).expect("empty capture-off samples satisfy the bound");
        assert_eq!(report.checks["sample_capacity"], "pass");
        report.samples.resize_with(SAMPLE_LIMIT, || Sample {
            ocr_ns: 1,
            frame_age_ns: None,
            cooldown_gap_ns: None,
            stream_id: 1,
            sequence: 0,
            epoch: 0,
            geometry_revision: 0,
        });
        sample_capacity(&mut report)
            .expect("exactly the bound remains a valid completed sample set");
        assert_eq!(report.checks["sample_capacity"], "pass");
        assert!(admit(&OperationContext::new(), &mut report).is_err());
        assert_eq!(report.metrics.ocr_admissions, 0);
        assert_eq!(report.samples.len(), SAMPLE_LIMIT);
    }
}
