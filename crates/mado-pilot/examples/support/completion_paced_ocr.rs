//! Private, model-free apparatus for this executable's explicit smoke mode and tests.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_pilot::{
    CapturePacingReport, Clock, Continuity, CoordinateSupport, Engine, Error, Frame,
    FrameDescriptor, FrameOrder, FrameRequest, FrameSelection, FrameStamp, Lifecycle,
    MonotonicInstant, OcrBackend, OcrBackendDescriptor, OcrBackendRequest, OcrCandidateSink,
    OcrRegion, OpenRequest, OperationContext, PacingUnsupportedReason, PixelExtent, PixelFormat,
    ProviderId, ResolvedCapturePacing, Result, Session, SessionDescription, Status,
    TargetDescription, TargetId,
};
use mado_pilot_runtime::{EngineWiring, IdentityIssuer, Matcher, OcrRecognizer, PackageLoader};
use mado_pilot_testkit::{
    CaptureProvider, CaptureSession, ControlledMatcher, ControlledOcr, ManualClock, Publication,
    ScriptedOcrCandidate, StreamState,
};

use super::{
    Consumption, Decision, WAIT_SLICE, close_session, consume, interpret_until, with_cleanup,
};

const EXTENT: PixelExtent = PixelExtent::new(32, 24);
const PROVIDER: ProviderId = ProviderId::new("completion-paced-example");

// StreamState owns publication, ordering, geometry and terminal arbitration.
// Only its idle Condvar wait is replaced by deterministic clock advancement.
#[derive(Clone)]
struct Source {
    description: SessionDescription,
    state: Arc<StreamState>,
    clock: Arc<ManualClock>,
    acquisitions: Arc<Mutex<Vec<MonotonicInstant>>>,
    idle_waits: Arc<AtomicUsize>,
    #[cfg(test)]
    close_failure: Arc<Mutex<Option<Status>>>,
    #[cfg(test)]
    close_authorities: Arc<Mutex<Vec<(bool, Option<Duration>)>>>,
}

impl fmt::Debug for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Source")
            .field("state", &self.state)
            .finish()
    }
}

impl Source {
    fn publish(&self, extent: PixelExtent, fill: u8, continuity: Continuity) -> Result<FrameStamp> {
        let descriptor = FrameDescriptor::packed(extent, PixelFormat::Rgba8)?;
        let frame = self.state.publish(Publication {
            captured_at: self.clock.now(),
            descriptor,
            placement: None,
            continuity,
            pixels: vec![fill; descriptor.byte_len()].into_boxed_slice(),
        })?;
        Ok(frame.stamp())
    }

    fn acquisition_times(&self) -> Vec<MonotonicInstant> {
        self.acquisitions
            .lock()
            .expect("controlled acquisitions")
            .clone()
    }
}

impl CaptureProvider for Source {
    fn provider(&self) -> ProviderId {
        PROVIDER
    }

    fn discover(&self, operation: &OperationContext) -> Result<Vec<TargetDescription>> {
        super::checkpoint(operation)?;
        Ok(vec![TargetDescription::new(
            self.description.target(),
            "controlled-completion-ocr",
            EXTENT,
            PixelFormat::Rgba8,
            CoordinateSupport::frame_only(),
        )])
    }

    fn open(
        &self,
        target: TargetId,
        request: &OpenRequest,
        operation: &OperationContext,
    ) -> Result<Arc<dyn CaptureSession>> {
        super::checkpoint(operation)?;
        if target != self.description.target() {
            return Err(mado_pilot::CaptureFault::UnknownTarget.into());
        }
        if request
            .required_format()
            .is_some_and(|format| format != PixelFormat::Rgba8)
        {
            return Err(mado_pilot::CaptureFault::UnsupportedOption.into());
        }
        let pacing = CapturePacingReport::unsupported(
            request
                .capture_pacing()
                .resolve(ResolvedCapturePacing::source_default()),
            PacingUnsupportedReason::SourceCannotPace,
        )?;
        let mut opened = self.clone();
        opened.description = opened.description.with_capture_pacing(pacing);
        Ok(Arc::new(opened))
    }
}

impl CaptureSession for Source {
    fn description(&self) -> SessionDescription {
        self.description.clone()
    }

    fn frame(&self, request: &FrameRequest, operation: &OperationContext) -> Result<Frame> {
        self.acquisitions
            .lock()
            .expect("controlled acquisitions")
            .push(self.clock.now());
        loop {
            super::checkpoint(operation)?;
            let ready = self
                .state
                .current()
                .is_some_and(|frame| match request.selection() {
                    FrameSelection::Latest => true,
                    FrameSelection::NewerThan(stamp) => {
                        matches!(frame.stamp().order(&stamp), Ok(FrameOrder::After) | Err(_))
                    }
                });
            if ready || self.state.lifecycle() != Lifecycle::Open {
                return self.state.frame(request, operation);
            }
            self.idle_waits.fetch_add(1, Ordering::Relaxed);
            self.clock.advance(WAIT_SLICE);
        }
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        #[cfg(test)]
        {
            self.close_authorities
                .lock()
                .expect("controlled close authority")
                .push((operation.cancellation().is_some(), operation.remaining()));
            if let Some(status) = *self.close_failure.lock().expect("controlled close failure") {
                return Err(Error::new(status, "controlled capture close failed"));
            }
        }
        self.state.drain(operation)
    }

    fn lifecycle(&self) -> Lifecycle {
        self.state.lifecycle()
    }
}

struct Backend {
    ocr: ControlledOcr,
    #[cfg(test)]
    on_admission: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl fmt::Debug for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Backend")
            .field("ocr", &self.ocr)
            .finish()
    }
}

impl OcrBackend for Backend {
    fn descriptor(&self) -> OcrBackendDescriptor {
        self.ocr.descriptor()
    }

    fn recognize(
        &self,
        request: &OcrBackendRequest<'_>,
        output: &mut dyn OcrCandidateSink,
        operation: &OperationContext,
    ) -> Result<()> {
        #[cfg(test)]
        {
            let hook = self
                .on_admission
                .lock()
                .expect("controlled OCR admission")
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
        self.ocr.recognize(request, output, operation)
    }

    fn close(&self, operation: &OperationContext) -> Result<()> {
        self.ocr.close(operation)
    }
}

struct Fixture {
    // Keep the runtime alive while its session is being exercised.
    _engine: Engine,
    session: Arc<Session>,
    source: Arc<Source>,
    backend: Arc<Backend>,
    clock: Arc<ManualClock>,
}

impl Fixture {
    fn new(inference: Duration) -> Result<Self> {
        let clock = Arc::new(ManualClock::new());
        let issuer = IdentityIssuer::new();
        let target = issuer.issue_target(PROVIDER)?;
        let stream = issuer.issue_stream()?;
        let source = Arc::new(Source {
            description: SessionDescription::new(
                target,
                stream,
                EXTENT,
                PixelFormat::Rgba8,
                CoordinateSupport::frame_only(),
            ),
            state: Arc::new(StreamState::new(stream)),
            clock: Arc::clone(&clock),
            acquisitions: Arc::new(Mutex::new(Vec::new())),
            idle_waits: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            close_failure: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            close_authorities: Arc::new(Mutex::new(Vec::new())),
        });
        let backend = Arc::new(Backend {
            ocr: ControlledOcr::new(PixelFormat::Rgba8)
                .with_latency(Arc::clone(&clock), inference)
                .with_candidates(vec![ScriptedOcrCandidate::new(
                    b"READY".as_slice(),
                    [(1.0, 1.0), (6.0, 1.0), (6.0, 4.0), (1.0, 4.0)],
                    0.95,
                    0,
                )]),
            #[cfg(test)]
            on_admission: Mutex::new(None),
        });
        let engine = Engine::new(EngineWiring {
            engine: issuer.engine(),
            capture: source.clone(),
            matcher: Matcher::new(Arc::new(ControlledMatcher::new(PixelFormat::Rgba8))),
            loader: PackageLoader::new(),
            ocr: Some(OcrRecognizer::new(backend.clone())),
            input: None,
            permission: None,
        })?;
        let operation = OperationContext::new().with_clock(clock.clone());
        let session = Arc::new(engine.open(target, &OpenRequest::new(), &operation)?);
        Ok(Self {
            _engine: engine,
            session,
            source,
            backend,
            clock,
        })
    }

    fn operation(&self, timeout: Duration) -> Result<OperationContext> {
        OperationContext::new()
            .with_clock(self.clock.clone())
            .with_timeout(timeout)
    }
}

pub(super) fn smoke() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new(Duration::from_millis(30))?;
    fixture.source.publish(EXTENT, 7, Continuity::Continuous)?;
    let operation = fixture.operation(Duration::from_secs(1))?;
    let mut interpreted = 0;
    let mut published = false;
    let work = consume(
        &fixture.session,
        &fixture.backend.descriptor(),
        Consumption {
            cooldown: Duration::from_millis(10),
            region: OcrRegion::FullFrame,
        },
        &operation,
        |result, operation| {
            interpreted += 1;
            fixture.clock.advance(Duration::from_millis(4));
            if interpreted == 1 {
                Ok(Decision::Continue)
            } else {
                interpret_until(result, "READY", operation)
            }
        },
        |slice| {
            fixture.clock.advance(slice);
            if !published {
                fixture
                    .source
                    .publish(EXTENT, 8, Continuity::Continuous)
                    .expect("controlled live publication");
                published = true;
            }
        },
    );
    let completed = with_cleanup(work, close_session(&fixture.session, &operation))?;
    let acquisition_times = fixture.source.acquisition_times();
    if completed.observations != 2
        || fixture.backend.ocr.recognition_count() != 2
        || acquisition_times
            != [
                MonotonicInstant::ORIGIN,
                MonotonicInstant::from_origin(Duration::from_millis(44)),
            ]
        || fixture.clock.elapsed() != Duration::from_millis(78)
        || !fixture.session.is_closed()
    {
        return Err(Error::new(
            Status::Internal,
            "controlled completion-pacing contract failed",
        )
        .into());
    }
    println!(
        "completion-paced-ocr controlled-smoke: observations=2 additional-cooldown-ms=10 logical-close=complete; no native capture, model, or input executed"
    );
    Ok(())
}

#[cfg(test)]
#[path = "completion_paced_ocr_tests.rs"]
mod tests;
