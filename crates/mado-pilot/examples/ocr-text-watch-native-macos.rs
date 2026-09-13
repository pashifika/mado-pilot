//! Private, supervised task 7.3 consumer. See docs/ocr-text-watch-macos.md.
//! No fixture launch, input, activation, permission request, or fallback here.

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("semantic=not-run reason=unsupported-host resource=not-run cleanup=not-run");
    std::process::exit(2);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    // Panic payloads can include caller-controlled text; the private runner still
    // records the nonzero exit and never promotes missing reports to success.
    std::panic::set_hook(Box::new(|_| eprintln!("consumer-panicked")));
    if let Err(failure) = native::run() {
        eprintln!("consumer-failed reason={}", failure.label());
        std::process::exit(1);
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native {
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use mado_pilot::{
        ChangeDetectionPolicy, ClipPolicy, CoordinateSpace, CpuMapping, Engine, Frame, FrameOrder,
        FrameRequest, FrameStamp, NativeEngineRequest, OcrExecutionProvider, OcrProfile,
        OcrProfileConfig, OcrTextAnalysisRate, OcrTextQuery, OcrTextQueryOutcome,
        OcrTextQueryProgress, OcrTextStability, OcrTextTerminalOutcome, OcrTextWatchRequest,
        OcrTextWatchResult, OpenRequest, OperationContext, PermissionKind, PermissionState,
        PixelExtent, PixelFormat, PixelRect, Rect, Session, Status, TargetKind,
    };

    const LITERAL: &str = "魔導士";
    const TICK: Duration = Duration::from_millis(20);
    const SEMANTIC: Duration = Duration::from_secs(15);
    const CONTROL: Duration = Duration::from_secs(5);
    type Check<T> = Result<T, Failure>;

    pub(super) enum Failure {
        Rule(&'static str),
        Api(Status),
    }
    impl Failure {
        pub(super) fn label(&self) -> &'static str {
            match self {
                Self::Rule(label) => label,
                Self::Api(Status::DeadlineExceeded) => "deadline-exceeded",
                Self::Api(Status::Unsupported) => "unsupported",
                Self::Api(Status::TargetLost) => "target-lost",
                Self::Api(Status::InvalidArgument) => "invalid-argument",
                Self::Api(_) => "typed-api-failure",
            }
        }
    }
    fn api<T>(result: mado_pilot::Result<T>) -> Check<T> {
        result.map_err(|error| Failure::Api(error.status()))
    }
    fn require(value: bool, rule: &'static str) -> Check<()> {
        if value {
            Ok(())
        } else {
            Err(Failure::Rule(rule))
        }
    }
    fn context(duration: Duration) -> Check<OperationContext> {
        api(OperationContext::new().with_timeout(duration))
    }
    fn rect(right: f64, bottom: f64) -> Check<Rect> {
        Rect::new(CoordinateSpace::TargetLogical, 0.0, 0.0, right, bottom)
            .map_err(|_| Failure::Rule("fixed-geometry"))
    }
    fn read_small(path: &Path) -> Check<Option<String>> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Failure::Rule("control-read")),
        };
        let mut bytes = Vec::with_capacity(513);
        file.take(513)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::Rule("control-read"))?;
        require(bytes.len() <= 512, "control-size")?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| Failure::Rule("control-encoding"))
    }
    fn atomic_write(root: &Path, name: &str, value: &str) -> Check<()> {
        let temporary = root.join(format!("{name}.consumer-tmp"));
        fs::write(&temporary, value).map_err(|_| Failure::Rule("control-write"))?;
        fs::rename(temporary, root.join(name)).map_err(|_| Failure::Rule("control-publish"))
    }
    struct Evidence(File);
    impl Evidence {
        fn line(&mut self, text: impl AsRef<str>) -> Check<()> {
            writeln!(self.0, "{}", text.as_ref()).map_err(|_| Failure::Rule("evidence-write"))?;
            self.0.flush().map_err(|_| Failure::Rule("evidence-write"))
        }
    }
    struct Fixture {
        root: PathBuf,
        nonce: String,
        token: u64,
        pid: u32,
        window: i64,
    }
    impl Fixture {
        fn ack(&mut self, sequence: u32, state: &str, wait: Duration) -> Check<()> {
            let deadline = Instant::now() + wait;
            loop {
                require(
                    !self.root.join("fixture-failure").exists(),
                    "fixture-failed",
                )?;
                if let Some(row) = read_small(&self.root.join("ack"))? {
                    let words: Vec<_> = row.split_whitespace().collect();
                    require(
                        words.len() == 8 && words[0] == self.nonce,
                        "fixture-ack-identity",
                    )?;
                    let seq = words[1]
                        .parse::<u32>()
                        .map_err(|_| Failure::Rule("fixture-sequence"))?;
                    require(seq <= sequence, "fixture-sequence-ahead")?;
                    if seq == sequence {
                        require(words[2] == state, "fixture-state")?;
                        let pid = words[3]
                            .parse::<u32>()
                            .map_err(|_| Failure::Rule("fixture-pid"))?;
                        let window = words[4]
                            .parse::<i64>()
                            .map_err(|_| Failure::Rule("fixture-window"))?;
                        let Some(owner) = read_small(&self.root.join("fixture-owner"))? else {
                            require(Instant::now() < deadline, "fixture-owner-timeout")?;
                            thread::sleep(TICK);
                            continue;
                        };
                        require(
                            owner.trim().parse::<u32>().ok() == Some(pid),
                            "fixture-child-ownership",
                        )?;
                        require(self.pid == 0 || self.pid == pid, "fixture-pid-changed")?;
                        require(
                            self.window == 0 || self.window == window,
                            "fixture-window-changed",
                        )?;
                        let expected = if sequence >= 9 {
                            ("240", "140")
                        } else if sequence >= 6 {
                            ("520", "300")
                        } else {
                            ("480", "278")
                        };
                        require(
                            (words[5], words[6]) == expected && words[7] == "2.0",
                            "fixture-retina-geometry",
                        )?;
                        self.pid = pid;
                        if sequence < 10 {
                            require(window > 0, "fixture-window")?;
                            self.window = window;
                        }
                        return Ok(());
                    }
                }
                require(Instant::now() < deadline, "fixture-ack-timeout")?;
                thread::sleep(TICK);
            }
        }
        fn command(&mut self, sequence: u32, command: &str) -> Check<()> {
            atomic_write(
                &self.root,
                "command",
                &format!("{} {sequence} {command}\n", self.nonce),
            )?;
            self.ack(sequence, command, CONTROL)
        }
        fn marker(&self, mapping: &CpuMapping, state: u32) -> Check<bool> {
            let descriptor = mapping.descriptor();
            let width = descriptor.extent().width() as usize;
            let height = descriptor.extent().height() as usize;
            require(width >= 384 && height >= 16, "marker-extent")?;
            let stride = descriptor.stride();
            for bit in 0..96 {
                let one = if bit < 64 {
                    (self.token >> (63 - bit)) & 1 != 0
                } else {
                    (state >> (95 - bit)) & 1 != 0
                };
                let offset = (height - 8) * stride + (bit * 4 + 2) * 4;
                let pixel = &mapping.bytes()[offset..offset + 4]; // BGRA mapping.
                let observed = if pixel[0] < 40 && pixel[1] > 215 && pixel[2] > 215 {
                    true
                } else if pixel[0] > 215 && pixel[1] < 40 && pixel[2] < 40 {
                    false
                } else {
                    return Ok(false);
                };
                if one != observed {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
    fn map(frame: &Frame) -> Check<CpuMapping> {
        api(frame.map(PixelFormat::Bgra8, &context(CONTROL)?))
    }
    fn newer(stamp: FrameStamp, checkpoint: FrameStamp) -> bool {
        stamp.order(&checkpoint) == Ok(FrameOrder::After)
    }
    // Only verification checkpoints/ownership pixels use frame acquisition.
    // Recognition and its confirmation are exclusively maintained public queries.
    fn source(
        session: &Session,
        fixture: &Fixture,
        state: u32,
        after: Option<FrameStamp>,
    ) -> Check<Frame> {
        let operation = context(CONTROL)?;
        let mut request = after.map_or_else(FrameRequest::latest, FrameRequest::newer_than);
        loop {
            let frame =
                api(session.acquire_frame(&request, &operation)).map_err(
                    |failure| match failure {
                        Failure::Api(Status::DeadlineExceeded) => {
                            Failure::Rule("producer-progress-absent-frame")
                        }
                        other => other,
                    },
                )?;
            let mapping = api(frame.map(PixelFormat::Bgra8, &operation))?;
            if fixture.marker(&mapping, state)? {
                return Ok(frame);
            }
            request = FrameRequest::newer_than(frame.stamp());
        }
    }
    fn start(session: &Session, roi: Rect, confirmations: u32) -> Check<OcrTextQuery> {
        api(session.start_ocr_text_watch(api(OcrTextWatchRequest::new(
            roi,
            ClipPolicy::Reject,
            LITERAL,
            0.0,
            CoordinateSpace::TargetLogical,
            api(OcrTextAnalysisRate::from_minimum_interval(
                Duration::from_millis(250),
            ))?,
            api(OcrTextStability::consecutive(confirmations))?,
            ChangeDetectionPolicy::ExactRgba,
            context(Duration::from_secs(100))?,
        ))?))
    }
    fn progress(
        query: &OcrTextQuery,
        predicate: impl Fn(OcrTextQueryProgress) -> bool,
    ) -> Check<OcrTextQueryProgress> {
        let deadline = Instant::now() + SEMANTIC;
        loop {
            match query.poll() {
                OcrTextQueryOutcome::Pending(value) if predicate(value) => return Ok(value),
                OcrTextQueryOutcome::Pending(_) => {}
                OcrTextQueryOutcome::Terminal(_) => {
                    return Err(Failure::Rule("unexpected-terminal"));
                }
            }
            require(Instant::now() < deadline, "semantic-progress-timeout")?;
            thread::sleep(TICK);
        }
    }
    fn first_positive(query: &OcrTextQuery) -> Check<()> {
        let deadline = Instant::now() + SEMANTIC;
        loop {
            match query.poll() {
                OcrTextQueryOutcome::Pending(value) if value.confirmed_observations() > 0 => {
                    return Ok(());
                }
                OcrTextQueryOutcome::Pending(_) => {}
                OcrTextQueryOutcome::Terminal(outcome) => {
                    matched(outcome.as_ref())?;
                    return Ok(());
                }
            }
            require(Instant::now() < deadline, "producer-progress-no-text-frame")?;
            thread::sleep(TICK);
        }
    }
    fn matched(outcome: &OcrTextTerminalOutcome) -> Check<&OcrTextWatchResult> {
        match outcome {
            OcrTextTerminalOutcome::Matched(result) => Ok(result),
            OcrTextTerminalOutcome::DeadlineExceeded => {
                Err(Failure::Rule("producer-progress-no-text-frame"))
            }
            OcrTextTerminalOutcome::Failed(error) => Err(Failure::Api(error.status())),
            _ => Err(Failure::Rule("expected-match")),
        }
    }
    fn oracle(
        result: &OcrTextWatchResult,
        checkpoint: FrameStamp,
        width: u32,
        height: u32,
    ) -> Check<()> {
        let ocr = result.result();
        let frame = result.frame();
        require(
            newer(frame.stamp(), checkpoint),
            "pre-transition-source-reused",
        )?;
        require(
            frame.stamp().epoch() == checkpoint.epoch()
                && frame.stamp().geometry() == checkpoint.geometry(),
            "unexpected-source-successor",
        )?;
        require(
            frame.stamp() == ocr.stamp() && frame.transform() == ocr.transform(),
            "result-source-correlation",
        )?;
        require(
            frame.descriptor().extent() == PixelExtent::new(width, height),
            "capture-extent",
        )?;
        let placement = frame
            .transform()
            .target()
            .ok_or(Failure::Rule("unsupported-target-transform"))?;
        require(
            placement.scale().x() == 2.0 && placement.scale().y() == 2.0,
            "retina-scale",
        )?;
        require(
            ocr.output_space() == CoordinateSpace::TargetLogical,
            "output-space",
        )?;
        let expected =
            PixelRect::new(0, 0, 960, 540).map_err(|_| Failure::Rule("fixed-geometry"))?;
        require(ocr.effective_region() == expected, "effective-roi")?;
        require(
            frame
                .transform()
                .resolve_capture_pixels(rect(480.0, 270.0)?, ClipPolicy::Reject)
                .ok()
                == Some(expected),
            "retina-roi-projection",
        )?;
        require(
            ocr.regions().len() == 8 && result.satisfying_region_indexes() == [0],
            "fixed-region-oracle",
        )?;
        require(ocr.regions()[0].text() == LITERAL, "fixed-literal-oracle")?;
        for region in ocr.regions() {
            let confidence = region.confidence().get();
            require(
                confidence.is_finite() && (0.0..=1.0).contains(&confidence),
                "confidence-oracle",
            )?;
            for point in region.geometry().points() {
                require(
                    point.space() == CoordinateSpace::TargetLogical,
                    "region-point-output-space",
                )?;
            }
        }
        let mut left = f64::INFINITY;
        let mut top = f64::INFINITY;
        let mut right = f64::NEG_INFINITY;
        let mut bottom = f64::NEG_INFINITY;
        for point in ocr.regions()[0].geometry().points() {
            let pixel = frame
                .transform()
                .convert_point(point, CoordinateSpace::CapturePixels)
                .map_err(|_| Failure::Rule("retina-result-projection"))?;
            require(
                pixel.space() == CoordinateSpace::CapturePixels
                    && (pixel.x() - point.x() * 2.0).abs() < 1e-6
                    && (pixel.y() - point.y() * 2.0).abs() < 1e-6,
                "fixed-retina-projection",
            )?;
            let roundtrip = frame
                .transform()
                .convert_point(pixel, CoordinateSpace::TargetLogical)
                .map_err(|_| Failure::Rule("retina-result-projection"))?;
            require(
                (roundtrip.x() - point.x()).abs() < 1e-6
                    && (roundtrip.y() - point.y()).abs() < 1e-6,
                "retina-roundtrip",
            )?;
            // Compare raw returned logical coordinates with the independent
            // half-size fixture oracle, not geometry converted by the product.
            left = left.min(point.x());
            top = top.min(point.y());
            right = right.max(point.x());
            bottom = bottom.max(point.y());
        }
        let intersection = (right.min(101.5) - left.max(26.5)).max(0.0)
            * (bottom.min(56.0) - top.max(30.0)).max(0.0);
        let union = (right - left) * (bottom - top) + 75.0 * 26.0 - intersection;
        require(
            union > 0.0 && intersection / union >= 0.5,
            "fixed-geometry-iou",
        )?;
        require(
            ((left + right) / 2.0 - 64.0).abs() <= 12.0
                && ((top + bottom) / 2.0 - 43.0).abs() <= 6.75,
            "fixed-geometry-center",
        )
    }
    fn quiescent(engine: &Engine) -> Check<()> {
        let deadline = Instant::now() + CONTROL;
        loop {
            let value = engine.ocr_text_observation();
            if value.physical_ocr_in_flight == 0
                && value.logical_ocr_in_flight == 0
                && value.template_mapping_reservations == 0
                && !value.mapping_barrier
            {
                return Ok(());
            }
            require(Instant::now() < deadline, "physical-work-not-quiescent")?;
            thread::sleep(TICK);
        }
    }
    pub(super) fn run() -> Check<()> {
        require(
            std::env::var("MADO_PILOT_OCR_PRIVATE_SUPERVISED").as_deref() == Ok("1"),
            "supervisor-required",
        )?;
        let mut args = std::env::args().skip(1);
        let root = PathBuf::from(args.next().ok_or(Failure::Rule("missing-control-root"))?);
        let nonce = args.next().ok_or(Failure::Rule("missing-fixture-token"))?;
        let token = u64::from_str_radix(&nonce, 16).map_err(|_| Failure::Rule("fixture-token"))?;
        require(
            nonce.len() == 16 && args.next().is_none(),
            "consumer-arguments",
        )?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("consumer.report"))
            .map_err(|_| Failure::Rule("evidence-create"))?;
        let mut evidence = Evidence(file);
        let config = OcrProfileConfig::new(
            OcrProfile::BoundedDetector,
            std::env::var_os("MADO_PILOT_G004_MODEL_ROOT")
                .ok_or(Failure::Rule("model-prerequisite"))?,
            std::env::var_os("MADO_PILOT_ONNX_RUNTIME")
                .ok_or(Failure::Rule("runtime-prerequisite"))?,
        );
        let engine = api(mado_pilot::macos_engine_with_ocr_profile(
            NativeEngineRequest::new(),
            &config,
            &context(Duration::from_secs(45))?,
        ))?;
        let permission = api(engine.permission(PermissionKind::ScreenCapture, &context(CONTROL)?))?;
        let decision = match permission.state() {
            PermissionState::Granted => "granted",
            PermissionState::NotGranted => "denied-or-undetermined",
            PermissionState::Unavailable => "unsupported",
            PermissionState::Unknown => "undetermined",
            _ => "undetermined",
        };
        evidence.line(format!("permission={decision} native={permission:?}"))?;
        atomic_write(&root, "permission", decision)?;
        if !permission.is_granted() {
            evidence.line("semantic=not-run resource=not-run cleanup=not-run")?;
            return Err(Failure::Rule("screen-recording-not-granted"));
        }
        let backend = engine
            .ocr_backend()
            .ok_or(Failure::Rule("ocr-unavailable"))?;
        let provider = engine
            .ocr_provider()
            .ok_or(Failure::Rule("provider-unavailable"))?;
        require(
            backend.profile().as_str() == mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
                && provider.active_provider() == OcrExecutionProvider::Cpu
                && !provider.initialization_fell_back(),
            "initialized-cpu-profile",
        )?;
        evidence.line(format!("backend={backend:?} provider={provider:?}"))?;
        let mut fixture = Fixture {
            root,
            nonce,
            token,
            pid: 0,
            window: 0,
        };
        fixture.ack(0, "blank", Duration::from_secs(10))?;
        let title = format!("MadoPilot OCR private {}", fixture.nonce);
        let targets = api(engine.discover(&context(CONTROL)?))?;
        let mut selected = targets.iter().filter(|target| {
            target.name() == title && target.capability().kind() == Some(TargetKind::Window)
        });
        let target = selected
            .next()
            .ok_or(Failure::Rule("owned-window-absent"))?;
        require(selected.next().is_none(), "owned-window-ambiguous")?;
        let session = api(engine.open(target.id(), &OpenRequest::new(), &context(CONTROL)?))?;
        evidence.line(format!(
            "fixture_pid={} fixture_window={} target={} stream={}",
            fixture.pid,
            fixture.window,
            session.target(),
            session.stream()
        ))?;
        let mut retained: Option<Arc<OcrTextTerminalOutcome>> = None;
        let mut retained_mapping: Option<CpuMapping> = None;
        let mut retained_frame: Option<Frame> = None;
        let mut retained_ocr: Option<mado_pilot::OcrResult> = None;
        let mut stage = "acknowledged-transition";
        let semantic = (|| -> Check<()> {
            let query = start(&session, rect(480.0, 270.0)?, 2)?;
            fixture.command(1, "arm-blank")?;
            let initial = source(&session, &fixture, 1, None)?;
            let checkpoint = initial.stamp();
            let blank = map(&initial)?;
            require(
                blank.descriptor().extent() == PixelExtent::new(960, 556),
                "initial-retina-extent",
            )?;
            for y in 0..540 {
                let row = &blank.bytes()[y * blank.descriptor().stride()..][..960 * 4];
                require(
                    row.chunks_exact(4)
                        .all(|pixel| pixel.iter().all(|channel| *channel >= 250)),
                    "initial-not-blank",
                )?;
            }
            drop(blank);
            drop(initial);
            evidence.line(format!("checkpoint={checkpoint} acknowledged_state=1"))?;
            match query.wait(&context(Duration::from_millis(100))?) {
                Err(error) if error.status() == Status::DeadlineExceeded => {}
                _ => return Err(Failure::Rule("independent-wait-authority")),
            }
            require(
                matches!(query.poll(), OcrTextQueryOutcome::Pending(_)),
                "blank-query-not-pending",
            )?;
            fixture.command(2, "show")?;
            first_positive(&query)?;
            // A separate acknowledged marker pulse supplies a distinct source even
            // if ScreenCaptureKit coalesces repeated presentations of the image.
            fixture.command(3, "pulse")?;
            let outcome =
                api(query.wait(&context(SEMANTIC)?)).map_err(|failure| match failure {
                    Failure::Api(Status::DeadlineExceeded) => {
                        Failure::Rule("producer-progress-no-text-frame")
                    }
                    other => other,
                })?;
            let result = matched(outcome.as_ref())?;
            oracle(result, checkpoint, 960, 556)?;
            require(
                result.result().backend() == &backend,
                "initialized-result-identity",
            )?;
            require(
                result.confirmed_observations() == 2
                    && newer(result.first_confirmed_frame(), checkpoint)
                    && newer(result.frame().stamp(), result.first_confirmed_frame()),
                "distinct-positive-observations",
            )?;
            let mapping = map(result.frame())?;
            require(
                fixture.marker(&mapping, 2)? || fixture.marker(&mapping, 3)?,
                "match-not-acknowledged-state",
            )?;
            evidence.line(format!(
                "row=acknowledged-transition status=passed frame={} first={}",
                result.frame().stamp(),
                result.first_confirmed_frame()
            ))?;
            retained_frame = Some(result.frame().clone());
            retained_ocr = Some(result.result().clone());
            retained_mapping = Some(mapping);
            retained = Some(outcome);
            drop(query);

            stage = "retained-producer-progress";
            fixture.command(4, "blank")?;
            let old = retained_frame
                .as_ref()
                .ok_or(Failure::Rule("retained-frame-missing"))?
                .stamp();
            let blank = source(&session, &fixture, 4, Some(old))?;
            evidence.line(format!(
                "row=retained-producer-progress status=passed newer={}",
                blank.stamp()
            ))?;
            let retained_checkpoint = blank.stamp();
            drop(blank);

            stage = "resize-reset";
            let resizing = start(&session, rect(480.0, 270.0)?, 2)?;
            fixture.command(5, "show")?;
            let shown = source(&session, &fixture, 5, Some(retained_checkpoint))?;
            let before = progress(&resizing, |value| {
                value.confirmed_observations() > 0
                    && value.pending_count() == 0
                    && value.in_flight_count() == 0
                    && value.physical_in_flight_count() == 0
            })?;
            require(
                before.confirmed_observations() == 1,
                "resize-initial-confirmation-count",
            )?;
            let before_stamp = before
                .last_accepted_frame()
                .ok_or(Failure::Rule("accepted-source-missing"))?;
            require(
                (before_stamp == shown.stamp() || newer(before_stamp, shown.stamp()))
                    && before_stamp.epoch() == shown.stamp().epoch()
                    && before_stamp.geometry() == shown.stamp().geometry(),
                "resize-initial-source",
            )?;
            let completed = mado_pilot::OcrTextWorkDisposition::Completed;
            let before_completed = before.work().get(completed);
            // Keep the identical positive ROI through resize. Inspect the FIRST
            // completion change: a negative or extra accepted analysis fails,
            // rather than waiting until confirmations happen to become one.
            fixture.command(6, "resize-show")?;
            let reset = progress(&resizing, |value| {
                value.work().get(completed) != before_completed
            })?;
            require(
                before_completed.checked_add(1) == Some(reset.work().get(completed))
                    && reset.confirmed_observations() == 1,
                "resize-first-successor-positive",
            )?;
            let reset_stamp = reset
                .last_accepted_frame()
                .ok_or(Failure::Rule("resize-accepted-source-missing"))?;
            require(
                newer(reset_stamp, before_stamp)
                    && reset_stamp.geometry() != before_stamp.geometry()
                    && reset_stamp.epoch() > before_stamp.epoch(),
                "resize-source-not-successor",
            )?;
            let resized = source(&session, &fixture, 6, Some(before_stamp))?;
            require(
                reset_stamp.geometry() == resized.stamp().geometry()
                    && reset_stamp.epoch() == resized.stamp().epoch(),
                "resize-successor-authority",
            )?;
            evidence.line(format!("resize_before={before_stamp} completed_before={before_completed} resize_checkpoint={} reset={reset:?}", resized.stamp()))?;
            let ready = resizing.progress();
            require(
                ready.work().get(completed) == reset.work().get(completed)
                    && ready.last_accepted_frame() == Some(reset_stamp)
                    && ready.confirmed_observations() == 1,
                "resize-extra-pre-pulse-analysis",
            )?;
            fixture.command(7, "pulse")?;
            let successor_outcome = api(resizing.wait(&context(SEMANTIC)?))?;
            let successor_result = matched(successor_outcome.as_ref())?;
            oracle(successor_result, resized.stamp(), 1040, 600)?;
            require(
                successor_result.result().backend() == &backend,
                "initialized-result-identity",
            )?;
            require(
                successor_result.confirmed_observations() == 2
                    && successor_result.first_confirmed_frame() == reset_stamp
                    && newer(successor_result.frame().stamp(), reset_stamp)
                    && before_completed.checked_add(2)
                        == Some(resizing.progress().work().get(completed)),
                "resize-consecutive-successor-authority",
            )?;
            require(
                successor_result.frame().stamp().geometry() == reset_stamp.geometry()
                    && successor_result.frame().stamp().epoch() == reset_stamp.epoch()
                    && fixture.marker(&map(successor_result.frame())?, 7)?,
                "resize-result-authority",
            )?;
            evidence.line("row=resize-reset status=passed")?;
            let successor_stamp = successor_result.frame().stamp();
            drop(resizing);
            drop(successor_outcome);
            drop(resized);
            drop(shown);
            quiescent(&engine)?;

            stage = "resize-invalidity";
            fixture.command(8, "blank")?;
            let before_shrink = source(&session, &fixture, 8, Some(successor_stamp))?;
            let edge = Rect::new(CoordinateSpace::TargetLogical, 490.0, 280.0, 510.0, 290.0)
                .map_err(|_| Failure::Rule("fixed-edge-roi"))?;
            let invalid = start(&session, edge, 1)?;
            fixture.command(9, "shrink-blank")?;
            let shrunk = source(&session, &fixture, 9, Some(before_shrink.stamp()))?;
            require(
                shrunk.stamp().geometry() != before_shrink.stamp().geometry(),
                "shrink-geometry-unchanged",
            )?;
            let invalid_outcome = api(invalid.wait(&context(SEMANTIC)?))?;
            require(
                matches!(invalid_outcome.as_ref(), OcrTextTerminalOutcome::Failed(error)
                if error.status() == Status::InvalidArgument),
                "expected-invalid-region",
            )?;
            evidence.line(format!(
                "row=resize-invalidity status=passed frame={}",
                shrunk.stamp()
            ))?;
            drop(invalid);
            drop(invalid_outcome);
            drop(before_shrink);
            drop(shrunk);

            stage = "exact-window-loss";
            let lost = start(&session, rect(100.0, 100.0)?, 1)?;
            require(
                matches!(lost.poll(), OcrTextQueryOutcome::Pending(_)),
                "loss-query-not-pending",
            )?;
            fixture.command(10, "close-window")?;
            let loss = api(lost.wait(&context(SEMANTIC)?))?;
            require(
                matches!(loss.as_ref(), OcrTextTerminalOutcome::TargetLost),
                "expected-exact-target-loss",
            )?;
            evidence.line("row=exact-window-loss status=passed")?;
            drop(lost);
            drop(loss);
            Ok(())
        })();
        if let Err(failure) = &semantic {
            evidence.line(format!(
                "row={stage} status=failed reason={}",
                failure.label()
            ))?;
            if let Failure::Api(status) = failure {
                evidence.line(format!("failure_status={status:?}"))?;
            }
        }
        // Always attempt each endpoint once; a semantic failure cannot hide a
        // resource or cleanup failure, and a successful match cannot erase one.
        let close_started = Instant::now();
        let close = api(session.close(&context(CONTROL)?));
        let closed = close.is_ok() && session.is_closed() && close_started.elapsed() <= CONTROL;
        evidence.line(format!(
            "close={} close_ms={}",
            if closed { "passed" } else { "failed" },
            close_started.elapsed().as_millis()
        ))?;
        let resource = quiescent(&engine).is_ok();
        evidence.line(format!(
            "resource={} observations={:?}",
            if resource { "passed" } else { "failed" },
            engine.ocr_text_observation()
        ))?;
        drop(session);
        drop(engine);
        let retention = (|| -> Check<()> {
            let outcome = retained
                .as_ref()
                .ok_or(Failure::Rule("retained-result-not-run"))?;
            let result = matched(outcome.as_ref())?;
            let exact = retained_frame
                .as_ref()
                .ok_or(Failure::Rule("retained-frame-not-run"))?;
            let before = retained_mapping
                .as_ref()
                .ok_or(Failure::Rule("retained-map-not-run"))?;
            let after = map(result.frame())?;
            require(
                retained_ocr.as_ref() == Some(result.result()),
                "retained-ocr-mutated",
            )?;
            require(
                result.frame().stamp() == exact.stamp()
                    && after.stamp() == before.stamp()
                    && after.bytes() == before.bytes()
                    && result.result().regions()[0].text() == LITERAL
                    && result.satisfying_region_indexes() == [0],
                "retained-result-mutated",
            )?;
            evidence.line(format!("row=parent-close-retention status=passed frame={} unique_source_bytes={} mapped_bytes={}",
                exact.stamp(), exact.descriptor().byte_len(), before.bytes().len()))?;
            Ok(())
        })();
        if let Err(failure) = &retention {
            evidence.line(format!(
                "row=parent-close-retention status={} reason={}",
                if retained.is_some() {
                    "failed"
                } else {
                    "not-run"
                },
                failure.label()
            ))?;
        }
        drop(retained_mapping);
        drop(retained_frame);
        drop(retained_ocr);
        drop(retained);
        let fixture_exit = if semantic.is_ok() {
            fixture.command(11, "exit").is_ok()
        } else {
            atomic_write(&fixture.root, "stop", "stop\n").is_ok()
        };
        let passed = semantic.is_ok() && retention.is_ok();
        evidence.line(format!(
            "semantic={} resource={} cleanup={}",
            if passed { "passed" } else { "failed" },
            if resource { "passed" } else { "failed" },
            if closed && fixture_exit {
                "consumer-endpoints-passed"
            } else {
                "failed"
            }
        ))?;
        println!(
            "semantic={} resource={} cleanup=supervisor-pending",
            if passed { "passed" } else { "failed" },
            if resource { "passed" } else { "failed" }
        );
        if passed && resource && closed && fixture_exit {
            Ok(())
        } else {
            std::process::exit(1);
        }
    }
}
