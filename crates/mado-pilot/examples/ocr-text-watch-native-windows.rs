//! Private, explicitly authorized Windows WGC OCR procedure; see docs/ocr-text-watch-windows.md.
//! Run through the bounded target-owned runner, never as an unattended native test.

fn main() -> std::process::ExitCode {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        windows_procedure::run()
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        eprintln!(
            "ocr-text-watch-windows: semantic=not-run resource=not-run cleanup=not-run reason=unsupported-host"
        );
        std::process::ExitCode::from(2)
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod windows_procedure {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitCode, Stdio};
    use std::sync::{Arc, mpsc};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use mado_pilot::{
        ChangeDetectionPolicy, ClipPolicy, ContentDigest, CoordinateSpace, Engine, Frame,
        FrameOrder, FrameRequest, FrameStamp, NativeEngineRequest, OcrExecutionProvider,
        OcrExecutionProviderPolicy, OcrProfile, OcrProfileConfig, OcrTextAnalysisRate,
        OcrTextQuery, OcrTextQueryOutcome, OcrTextStability, OcrTextTerminalOutcome,
        OcrTextWatchRequest, OpenRequest, OperationContext, PixelFormat, Rect, Session,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
    };
    use windows::core::PCWSTR;

    type Checked<T> = Result<T, Box<dyn std::error::Error>>;
    const HUD_SHA: &str = "3ca2f418f6fb083e49a679638017609e11fe825ef4a1e6f56dc0e572c74f75e6";
    const TEXT: [&str; 8] = [
        "魔導士",
        "Lv.42",
        "HP1234/5678",
        "MP98%",
        "クエスト",
        "[A-7]",
        "次へ>",
        "READY!",
    ];
    const STEP: Duration = Duration::from_secs(20);
    const CLOSE: Duration = Duration::from_secs(5);
    const PROTOCOL: Duration = Duration::from_secs(10);
    const POLL: Duration = Duration::from_millis(10);

    fn require(value: bool, reason: &'static str) -> Checked<()> {
        if value { Ok(()) } else { Err(reason.into()) }
    }

    fn context(timeout: Duration) -> mado_pilot::Result<OperationContext> {
        OperationContext::new().with_timeout(timeout)
    }

    #[derive(Clone, Copy)]
    struct Ack {
        sequence: u8,
        width: u32,
        height: u32,
        state: u8,
        paints: u32,
    }

    struct Fixture {
        child: Child,
        responses: mpsc::Receiver<Result<String, &'static str>>,
        reader: Option<JoinHandle<()>>,
        hwnd: HWND,
        foreground: HWND,
        title: String,
        sequence: u8,
        paints: u32,
        forced: bool,
    }

    impl Fixture {
        fn start(executable: &Path, image: &Path, nonce: &str) -> Checked<Self> {
            // SAFETY: Read-only desktop observation; the returned HWND is never activated.
            let foreground = unsafe { GetForegroundWindow() };
            require(!foreground.is_invalid(), "foreground-unavailable")?;
            let child = Command::new(executable)
                .args(["--fixture"])
                .arg(image)
                .arg(nonce)
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: no child console activation.
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;
            let (sender, responses) = mpsc::sync_channel(1);
            let mut fixture = Self {
                child,
                responses,
                reader: None,
                hwnd: HWND::default(),
                foreground,
                title: format!("MadoPilot OCR private [{nonce}]"),
                sequence: 0,
                paints: 0,
                forced: false,
            };
            let mut stdout = fixture
                .child
                .stdout
                .take()
                .ok_or("fixture-stdout-unavailable")?;
            fixture.reader = Some(
                thread::Builder::new()
                    .name("ocr-fixture-replies".into())
                    .spawn(move || {
                        // One 192-byte line and one queued reply. No unbounded read_line or output queue.
                        loop {
                            let mut bytes = [0_u8; 192];
                            let mut used = 0;
                            let response = loop {
                                if used == bytes.len() {
                                    break Err("fixture-reply-too-long");
                                }
                                match stdout.read(&mut bytes[used..used + 1]) {
                                    Ok(1) if bytes[used] == b'\n' => {
                                        break std::str::from_utf8(&bytes[..used])
                                            .map(str::to_owned)
                                            .map_err(|_| "fixture-reply-not-utf8");
                                    }
                                    Ok(1) => used += 1,
                                    _ => break Err("fixture-reply-eof-or-read-failure"),
                                }
                            };
                            let failed = response.is_err();
                            if sender.send(response).is_err() || failed {
                                break;
                            }
                        }
                    })?,
            );
            Ok(fixture)
        }

        fn acknowledge(&mut self, evidence: &mut File) -> Checked<Ack> {
            let line = self.responses.recv_timeout(PROTOCOL)??;
            writeln!(evidence, "fixture_reply={line}")?;
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            require(
                fields.len() == 8 && fields[0] == "ACK",
                "fixture-reply-shape",
            )?;
            let sequence = fields[1].parse::<u8>()?;
            let pid = fields[2].parse::<u32>()?;
            let raw = fields[3].parse::<usize>()?;
            let hwnd = HWND(raw as *mut std::ffi::c_void);
            let ack = Ack {
                sequence,
                width: fields[4].parse()?,
                height: fields[5].parse()?,
                state: fields[6].parse()?,
                paints: fields[7].parse()?,
            };
            require(
                sequence == self.sequence && pid == self.child.id(),
                "fixture-reply-ownership",
            )?;
            if self.hwnd.is_invalid() {
                self.hwnd = hwnd;
            }
            require(hwnd == self.hwnd, "fixture-window-replaced")?;
            require(ack.state <= 2, "fixture-state-invalid")?;
            require(
                matches!((ack.width, ack.height), (960, 576) | (1024, 640)),
                "fixture-extent-invalid",
            )?;
            if ack.state < 2 {
                require(ack.paints > self.paints, "fixture-render-not-completed")?;
                self.check_window()?;
            } else {
                require(
                    // SAFETY: This exact HWND was returned by our still-owned child, used read-only.
                    !unsafe { IsWindow(Some(self.hwnd)) }.as_bool(),
                    "fixture-window-not-destroyed",
                )?;
            }
            self.paints = ack.paints;
            self.check_foreground()?;
            Ok(ack)
        }

        fn check_foreground(&self) -> Checked<()> {
            // SAFETY: Read-only observation; no focus or input API is called.
            require(
                unsafe { GetForegroundWindow() } == self.foreground,
                "foreground-changed",
            )
        }

        fn check_window(&mut self) -> Checked<()> {
            require(self.child.try_wait()?.is_none(), "fixture-process-ended")?;
            let mut pid = 0;
            let title: Vec<u16> = self.title.encode_utf16().chain(Some(0)).collect();
            let class: Vec<u16> = "MadoPilot.Private.OcrTextWatch.Windows.1"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: Both strings are NUL-terminated and live for the calls; HWND is only observed.
            let found = unsafe {
                GetWindowThreadProcessId(self.hwnd, Some(&mut pid));
                FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr()))?
            };
            require(
                pid == self.child.id() && found == self.hwnd,
                "exact-window-ownership-lost",
            )
        }

        fn command(&mut self, command: &'static str, evidence: &mut File) -> Checked<Ack> {
            require(self.sequence < 16, "fixture-command-bound")?;
            self.check_foreground()?;
            self.sequence += 1;
            // At most 16 commands of at most 8 bytes, one outstanding command at a time.
            // The outer Job deadline also bounds any stalled OS write or teardown.
            let stdin = self
                .child
                .stdin
                .as_mut()
                .ok_or("fixture-stdin-unavailable")?;
            writeln!(stdin, "{command}")?;
            self.acknowledge(evidence)
        }

        fn close(&mut self, evidence: &mut File) -> Checked<()> {
            let mut graceful = true;
            if self.child.try_wait()?.is_none() {
                if let Err(error) = self.command("quit", evidence) {
                    writeln!(evidence, "fixture_close_error={error:?}")?;
                    graceful = false;
                }
            } else {
                graceful = false;
            }
            self.child.stdin.take();
            let deadline = Instant::now() + CLOSE;
            let status = loop {
                if let Some(status) = self.child.try_wait()? {
                    break Some(status);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                thread::sleep(POLL);
            };
            if status.is_none() {
                self.forced = true;
                self.child.kill()?; // The retained Child handle, never PID/name lookup.
            }
            let reader_deadline = Instant::now() + CLOSE;
            while self
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.is_finished())
                && Instant::now() < reader_deadline
            {
                thread::sleep(POLL);
            }
            let drained = self.reader.as_ref().is_none_or(JoinHandle::is_finished);
            if drained && let Some(reader) = self.reader.take() {
                require(reader.join().is_ok(), "fixture-reader-panicked")?;
            }
            writeln!(
                evidence,
                "fixture_exit={status:?} forced={} reader_drained={drained}",
                self.forced
            )?;
            require(
                graceful && !self.forced && status.is_some_and(|code| code.success()) && drained,
                "fixture-cleanup-failed",
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Best-effort release only; explicit close and the outer Job own cleanup evidence.
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
        }
    }

    fn start_query(session: &Session, count: u32) -> Checked<OcrTextQuery> {
        Ok(session.start_ocr_text_watch(OcrTextWatchRequest::new(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 960.0, 540.0)?,
            ClipPolicy::Reject,
            TEXT[0],
            0.0,
            CoordinateSpace::FrameNormalized,
            OcrTextAnalysisRate::from_minimum_interval(Duration::from_secs(2))?,
            if count == 1 {
                OcrTextStability::immediate()
            } else {
                OcrTextStability::consecutive(count)?
            },
            ChangeDetectionPolicy::AnalysisAlways,
            context(Duration::from_secs(90))?,
        )?)?)
    }

    fn newer(before: FrameStamp, after: FrameStamp) -> bool {
        before.order(&after) == Ok(FrameOrder::Before)
    }

    fn pixel_state(
        frame: &Frame,
        ack: Ack,
        nonce: &[u8; 16],
        image: &[u8],
        operation: &OperationContext,
    ) -> Checked<bool> {
        let mapping = frame.map(PixelFormat::Bgra8, operation)?;
        let descriptor = mapping.descriptor();
        if descriptor.extent().width() != ack.width || descriptor.extent().height() != ack.height {
            return Ok(false);
        }
        let bytes = mapping.bytes();
        for y in 0..540 {
            for x in 0..960 {
                let at = y * descriptor.stride() + x * 4;
                let expected = if ack.state == 0 {
                    &[255, 255, 255][..]
                } else {
                    &image[(y * 960 + x) * 4..(y * 960 + x) * 4 + 3]
                };
                if &bytes[at..at + 3] != expected {
                    return Ok(false);
                }
            }
        }
        for (cell, expected) in nonce
            .iter()
            .copied()
            .chain([ack.sequence, ack.state])
            .enumerate()
        {
            let at = 548 * descriptor.stride() + (cell * 8 + 4) * 4;
            if bytes[at..at + 3] != [expected; 3] {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn checkpoint(
        session: &Session,
        before: Option<FrameStamp>,
        ack: Ack,
        nonce: &[u8; 16],
        image: &[u8],
        evidence: &mut File,
    ) -> Checked<FrameStamp> {
        let operation = context(STEP)?;
        let mut cursor = before;
        for _ in 0..128 {
            let request = cursor.map_or_else(FrameRequest::latest, FrameRequest::newer_than);
            let frame = session.acquire_frame(&request, &operation)?;
            if let Some(previous) = cursor {
                require(newer(previous, frame.stamp()), "source-not-newer")?;
            }
            cursor = Some(frame.stamp());
            // Transitional frames are observed, never retried as replacement semantic samples.
            if pixel_state(&frame, ack, nonce, image, &operation)? {
                writeln!(
                    evidence,
                    "checkpoint={} ack={} transform={:?}",
                    frame.stamp(),
                    ack.sequence,
                    frame.transform()
                )?;
                return Ok(frame.stamp());
            }
            writeln!(
                evidence,
                "transitional_source={} awaiting_ack={}",
                frame.stamp(),
                ack.sequence
            )?;
            require(
                operation.remaining().is_none_or(|left| !left.is_zero()),
                "checkpoint-deadline",
            )?;
        }
        Err("checkpoint-frame-bound".into())
    }

    fn accepted(
        query: &OcrTextQuery,
        source: FrameStamp,
        confirmations: u32,
        evidence: &mut File,
    ) -> Checked<FrameStamp> {
        let until = Instant::now() + STEP;
        loop {
            require(
                matches!(query.poll(), OcrTextQueryOutcome::Pending(_)),
                "premature-query-terminal",
            )?;
            let progress = query.progress();
            if let Some(frame) = progress.last_accepted_frame()
                && (frame == source || newer(source, frame))
                && frame.geometry() == source.geometry()
                && frame.epoch() == source.epoch()
            {
                require(
                    progress.confirmed_observations() == confirmations,
                    "confirmation-count-mismatch",
                )?;
                writeln!(
                    evidence,
                    "accepted={frame} confirmations={confirmations} progress={progress:?}"
                )?;
                return Ok(frame);
            }
            require(Instant::now() < until, "accepted-analysis-deadline")?;
            thread::sleep(POLL);
        }
    }

    struct Retained {
        outcome: Arc<OcrTextTerminalOutcome>,
        before: FrameStamp,
        ack: Ack,
        confirmations: u32,
        successor: bool,
        original: mado_pilot::OcrResult,
    }

    fn inspect(
        saved: &Retained,
        nonce: &[u8; 16],
        image: &[u8],
        evidence: &mut File,
    ) -> Checked<()> {
        let OcrTextTerminalOutcome::Matched(result) = saved.outcome.as_ref() else {
            return Err("expected-matched-outcome".into());
        };
        let frame = result.frame();
        let ocr = result.result();
        let (width, height) = if saved.successor {
            (1024_u32, 640_u32)
        } else {
            (960, 576)
        };
        require(
            (saved.ack.width, saved.ack.height) == (width, height),
            "scenario-extent-oracle",
        )?;
        require(
            newer(saved.before, frame.stamp()),
            "match-not-after-checkpoint",
        )?;
        if saved.successor {
            require(
                frame.stamp().geometry() != saved.before.geometry(),
                "resize-did-not-change-geometry",
            )?;
            require(
                result.first_confirmed_frame().geometry() == frame.stamp().geometry()
                    && result.first_confirmed_frame().epoch() == frame.stamp().epoch()
                    && newer(saved.before, result.first_confirmed_frame()),
                "resize-confirmation-not-reset",
            )?;
        } else {
            require(
                frame.stamp().epoch() == saved.before.epoch()
                    && frame.stamp().geometry() == saved.before.geometry(),
                "unexpected-source-transition",
            )?;
        }
        require(
            frame.stamp() == ocr.stamp() && frame.transform() == ocr.transform(),
            "result-source-mismatch",
        )?;
        require(ocr == &saved.original, "retained-ocr-output-changed")?;
        require(
            result.confirmed_observations() == saved.confirmations,
            "terminal-confirmations",
        )?;
        require(
            result.literal() == TEXT[0] && result.minimum_confidence() == 0.0,
            "retained-predicate-mismatch",
        )?;
        require(
            result.provider().active_provider() == OcrExecutionProvider::Cpu
                && result.provider().requested_policy() == OcrExecutionProviderPolicy::Cpu
                && !result.provider().initialization_fell_back()
                && result.provider().runtime_profile().as_str()
                    == mado_pilot::DEFAULT_OCR_RUNTIME_PROFILE_ID,
            "retained-provider-mismatch",
        )?;
        require(
            result.satisfying_region_indexes() == [0_u16],
            "satisfying-region-indexes",
        )?;
        require(
            ocr.regions().len() == 8 && ocr.output_space() == CoordinateSpace::FrameNormalized,
            "region-count-or-output-space",
        )?;
        let expected_roi = frame.transform().resolve_capture_pixels(
            Rect::new(CoordinateSpace::CapturePixels, 0.0, 0.0, 960.0, 540.0)?,
            ClipPolicy::Reject,
        )?;
        require(
            ocr.effective_region() == expected_roi,
            "effective-region-mismatch",
        )?;
        for (index, region) in ocr.regions().iter().enumerate() {
            let confidence = region.confidence().get();
            require(region.text() == TEXT[index], "fixed-region-text-mismatch")?;
            require(
                confidence.is_finite() && (0.0..=1.0).contains(&confidence),
                "invalid-confidence",
            )?;
            require(
                (region.text().contains(TEXT[0]) && confidence >= 0.0) == (index == 0),
                "within-one-region-predicate-mismatch",
            )?;
            for point in region.geometry().points() {
                require(
                    point.space() == CoordinateSpace::FrameNormalized,
                    "point-output-space",
                )?;
                let capture = frame
                    .transform()
                    .convert_point(point, CoordinateSpace::CapturePixels)?;
                require(
                    (capture.x() - point.x() * f64::from(width)).abs() < 1e-6
                        && (capture.y() - point.y() * f64::from(height)).abs() < 1e-6,
                    "output-coordinate-projection",
                )?;
            }
            writeln!(
                evidence,
                "region={index} text={:?} confidence={confidence:?} quad={:?}",
                region.text(),
                region.geometry()
            )?;
        }
        let points = ocr.regions()[0].geometry().points();
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in points {
            let capture = frame
                .transform()
                .convert_point(point, CoordinateSpace::CapturePixels)?;
            bounds[0] = bounds[0].min(capture.x());
            bounds[1] = bounds[1].min(capture.y());
            bounds[2] = bounds[2].max(capture.x());
            bounds[3] = bounds[3].max(capture.y());
        }
        let intersection = (bounds[2].min(203.0) - bounds[0].max(53.0)).max(0.0)
            * (bounds[3].min(112.0) - bounds[1].max(60.0)).max(0.0);
        let area = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
        require(
            intersection / (area + 150.0 * 52.0 - intersection) >= 0.5
                && ((bounds[0] + bounds[2]) / 2.0 - 128.0).abs() <= 960.0 * 0.025
                && ((bounds[1] + bounds[3]) / 2.0 - 86.0).abs() <= 540.0 * 0.025,
            "fixed-source-box-oracle",
        )?;
        require(
            pixel_state(frame, saved.ack, nonce, image, &context(STEP)?)?,
            "retained-frame-not-acknowledged-state",
        )?;
        writeln!(
            evidence,
            "matched_source={} first_confirmed={} backend={:?} source_bytes={}",
            frame.stamp(),
            result.first_confirmed_frame(),
            ocr.backend(),
            frame.descriptor().byte_len()
        )?;
        Ok(())
    }

    fn retain(
        query: &OcrTextQuery,
        before: FrameStamp,
        ack: Ack,
        confirmations: u32,
        successor: bool,
    ) -> Checked<Retained> {
        let outcome = query.wait(&context(STEP)?)?;
        let OcrTextTerminalOutcome::Matched(result) = outcome.as_ref() else {
            return Err(format!("query-terminal={outcome:?}").into());
        };
        let original = result.result().clone();
        require(
            Arc::ptr_eq(&outcome, &query.cancel()),
            "committed-terminal-replaced",
        )?;
        Ok(Retained {
            outcome,
            before,
            ack,
            confirmations,
            successor,
            original,
        })
    }

    fn scenarios(
        session: &Session,
        fixture: &mut Fixture,
        nonce: &[u8; 16],
        image: &[u8],
        retained: &mut Vec<Retained>,
        evidence: &mut File,
    ) -> Checked<()> {
        let first = start_query(session, 1)?;
        let blank = fixture.command("blank", evidence)?;
        let before = checkpoint(session, None, blank, nonce, image, evidence)?;
        accepted(&first, before, 0, evidence)?;
        let shown = fixture.command("show", evidence)?;
        retained.push(retain(&first, before, shown, 1, false)?);
        inspect(&retained[0], nonce, image, evidence)?;
        drop(first);
        writeln!(evidence, "transition=passed")?;

        let blank = fixture.command("blank", evidence)?;
        let before = checkpoint(
            session,
            Some(retained[0].original.stamp()),
            blank,
            nonce,
            image,
            evidence,
        )?;
        writeln!(evidence, "producer_progress_while_result_retained=passed")?;
        let resizing = start_query(session, 3)?;
        accepted(&resizing, before, 0, evidence)?;
        let shown = fixture.command("show", evidence)?;
        let shown_stamp = checkpoint(session, Some(before), shown, nonce, image, evidence)?;
        let pre_resize = accepted(&resizing, shown_stamp, 1, evidence)?;
        // Keep text identical: a negative frame cannot masquerade as a geometry reset.
        let resized = fixture.command("resize", evidence)?;
        let successor = checkpoint(session, Some(pre_resize), resized, nonce, image, evidence)?;
        require(
            successor.geometry() != pre_resize.geometry(),
            "resize-revision-not-observed",
        )?;
        accepted(&resizing, successor, 1, evidence)?;
        let tick = fixture.command("tick", evidence)?;
        let second = checkpoint(session, Some(successor), tick, nonce, image, evidence)?;
        accepted(&resizing, second, 2, evidence)?;
        let final_tick = fixture.command("tick", evidence)?;
        retained.push(retain(&resizing, pre_resize, final_tick, 3, true)?);
        inspect(&retained[1], nonce, image, evidence)?;
        drop(resizing);
        writeln!(evidence, "resize_reset=passed")?;

        let blank = fixture.command("blank", evidence)?;
        let before_loss = checkpoint(
            session,
            Some(retained[1].original.stamp()),
            blank,
            nonce,
            image,
            evidence,
        )?;
        let lost = start_query(session, 1)?;
        accepted(&lost, before_loss, 0, evidence)?;
        fixture.check_window()?;
        fixture.command("destroy", evidence)?;
        let terminal = lost.wait(&context(STEP)?)?;
        require(
            matches!(terminal.as_ref(), OcrTextTerminalOutcome::TargetLost),
            "expected-exact-target-loss",
        )?;
        require(
            Arc::ptr_eq(&terminal, &lost.cancel()),
            "target-loss-terminal-replaced",
        )?;
        writeln!(evidence, "target_loss=passed")?;
        Ok(())
    }

    fn quiescent(engine: &Engine, evidence: &mut File) -> Checked<()> {
        let until = Instant::now() + CLOSE;
        loop {
            let state = engine.ocr_text_observation();
            require(
                state.physical_ocr_high_water <= 1,
                "physical-ocr-capacity-exceeded",
            )?;
            if state.physical_ocr_in_flight == 0
                && state.logical_ocr_in_flight == 0
                && state.query_count == 0
                && state.template_mapping_reservations == 0
                && !state.mapping_barrier
            {
                writeln!(evidence, "physical_quiescence=passed observation={state:?}")?;
                return Ok(());
            }
            require(Instant::now() < until, "physical-quiescence-deadline")?;
            thread::sleep(POLL);
        }
    }

    fn memory(evidence: &mut File) -> Checked<()> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>())?,
            ..Default::default()
        };
        let size = counters.cb;
        // SAFETY: Writable correctly sized record and the current-process pseudo handle.
        unsafe {
            GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size)?;
        }
        writeln!(
            evidence,
            "working_set_bytes={} peak_working_set_bytes={}",
            counters.WorkingSetSize, counters.PeakWorkingSetSize
        )?;
        Ok(())
    }

    pub(super) fn run() -> ExitCode {
        match run_owned() {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(_) => {
                eprintln!(
                    "ocr-text-watch-windows: semantic=failed resource=unresolved cleanup=unresolved reason=procedure-refused"
                );
                ExitCode::from(2)
            }
        }
    }

    fn run_owned() -> Checked<bool> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        require(
            args.len() == 5 && args[0] == "--owned-wgc",
            "use-target-owned-runner",
        )?;
        let token = args[4].to_str().ok_or("nonce-not-unicode")?;
        require(
            token.len() == 32
                && token
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "nonce-invalid",
        )?;
        require(
            std::env::var("MADO_PILOT_OCR_WGC_RUN_NONCE")
                .ok()
                .as_deref()
                == Some(token),
            "runner-authority-absent",
        )?;
        let mut nonce = [0_u8; 16];
        for (i, value) in nonce.iter_mut().enumerate() {
            *value = u8::from_str_radix(&token[i * 2..i * 2 + 2], 16)?;
        }
        let image_path = PathBuf::from(&args[2]).canonicalize()?;
        require(
            std::fs::metadata(&image_path)?.len() == 2_073_600,
            "hud-byte-length",
        )?;
        let image = std::fs::read(&image_path)?;
        let digest = ContentDigest::of(&image);
        let actual: String = digest
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        require(actual == HUD_SHA, "hud-digest-mismatch")?;
        let mut evidence = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[3])?;
        writeln!(
            evidence,
            "profile=ocr-text-watch-windows-v1 nonce={token} hud_sha256={actual}"
        )?;
        let mut fixture = None;
        let mut engine = None;
        let mut session = None;
        let mut retained = Vec::with_capacity(2);
        let semantic = (|| -> Checked<()> {
            let model = PathBuf::from(
                std::env::var_os("MADO_PILOT_G004_MODEL_ROOT").ok_or("model-path-absent")?,
            )
            .canonicalize()?;
            let runtime = PathBuf::from(
                std::env::var_os("MADO_PILOT_ONNX_RUNTIME").ok_or("runtime-path-absent")?,
            )
            .canonicalize()?;
            writeln!(evidence, "model_root={model:?} runtime={runtime:?}")?;
            engine = Some(mado_pilot::windows_engine_with_ocr_profile(
                NativeEngineRequest::new(),
                &OcrProfileConfig::new(OcrProfile::BoundedDetector, model, runtime),
                &context(Duration::from_secs(60))?,
            )?);
            let engine = engine.as_ref().ok_or("engine-absent")?;
            let descriptor = engine.ocr_backend().ok_or("ocr-descriptor-absent")?;
            let provider = engine.ocr_provider().ok_or("ocr-provider-absent")?;
            require(
                descriptor.profile().as_str() == mado_pilot::ACCEPTED_BOUNDED_PROFILE_ID
                    && provider.requested_policy() == OcrExecutionProviderPolicy::Cpu
                    && provider.active_provider() == OcrExecutionProvider::Cpu
                    && !provider.initialization_fell_back(),
                "explicit-cpu-profile-mismatch",
            )?;
            writeln!(
                evidence,
                "initialized_backend={descriptor:?} initialized_provider={provider:?}"
            )?;
            memory(&mut evidence)?;
            fixture = Some(Fixture::start(Path::new(&args[1]), &image_path, token)?);
            let fixture = fixture.as_mut().ok_or("fixture-absent")?;
            let ready = fixture.acknowledge(&mut evidence)?;
            require(
                ready.sequence == 0
                    && ready.state == 0
                    && ready.width == 960
                    && ready.height == 576,
                "fixture-not-initially-blank",
            )?;
            let targets = engine.discover(&context(PROTOCOL)?)?;
            let mut exact = targets
                .iter()
                .filter(|target| target.name() == fixture.title);
            let target = exact.next().ok_or("owned-target-not-discovered")?;
            require(exact.next().is_none(), "owned-target-ambiguous")?;
            fixture.check_window()?;
            session = Some(engine.open(target.id(), &OpenRequest::new(), &context(PROTOCOL)?)?);
            fixture.check_window()?;
            let session = session.as_ref().ok_or("session-absent")?;
            writeln!(
                evidence,
                "target={:?} session={:?}",
                target.id(),
                session.description()
            )?;
            scenarios(
                session,
                fixture,
                &nonce,
                &image,
                &mut retained,
                &mut evidence,
            )?;
            for saved in &retained {
                let OcrTextTerminalOutcome::Matched(result) = saved.outcome.as_ref() else {
                    return Err("retained-match-absent".into());
                };
                require(
                    result.target() == session.target()
                        && result.result().backend() == &descriptor
                        && result.provider() == &provider,
                    "initialized-result-identity-mismatch",
                )?;
            }
            Ok(())
        })();
        writeln!(
            evidence,
            "semantic={}",
            if semantic.is_ok() { "passed" } else { "failed" }
        )?;
        if let Err(error) = &semantic {
            writeln!(evidence, "semantic_error={error:?}")?;
        }
        let resource = if let Some(engine) = &engine {
            quiescent(engine, &mut evidence)
        } else {
            Err("engine-unavailable".into())
        };
        if let Err(error) = &resource {
            writeln!(evidence, "resource_error={error:?}")?;
        }
        let mut cleanup_ok = true;
        if let Some(session) = session.take() {
            let start = Instant::now();
            let closed = session.close(&context(CLOSE)?);
            cleanup_ok &= closed.is_ok() && start.elapsed() <= CLOSE;
            writeln!(
                evidence,
                "session_close={closed:?} elapsed_ms={}",
                start.elapsed().as_millis()
            )?;
            drop(session);
        }
        let start = Instant::now();
        drop(engine.take());
        cleanup_ok &= start.elapsed() <= CLOSE;
        writeln!(
            evidence,
            "engine_release_ms={}",
            start.elapsed().as_millis()
        )?;
        let mut retention_ok = retained.len() == 2;
        for saved in &retained {
            if let Err(error) = inspect(saved, &nonce, &image, &mut evidence) {
                retention_ok = false;
                writeln!(evidence, "retained_error={error:?}")?;
            }
        }
        writeln!(
            evidence,
            "retained_after_parents={}",
            if retention_ok { "passed" } else { "failed" }
        )?;
        let retained_bytes: usize = retained
            .iter()
            .map(|saved| match saved.outcome.as_ref() {
                OcrTextTerminalOutcome::Matched(result) => result.frame().descriptor().byte_len(),
                _ => 0,
            })
            .sum();
        writeln!(
            evidence,
            "caller_retained_results={} caller_source_bytes={retained_bytes}",
            retained.len()
        )?;
        for saved in &retained {
            if let OcrTextTerminalOutcome::Matched(result) = saved.outcome.as_ref() {
                writeln!(
                    evidence,
                    "retained_logical_extent={:?}",
                    result.retained_extent()
                )?;
            }
        }
        drop(retained);
        let measured = memory(&mut evidence);
        if let Err(error) = &measured {
            writeln!(evidence, "memory_error={error:?}")?;
        }
        if let Some(fixture) = &mut fixture
            && let Err(error) = fixture.close(&mut evidence)
        {
            cleanup_ok = false;
            writeln!(evidence, "cleanup_error={error:?}")?;
        }
        let semantic_ok = semantic.is_ok() && retention_ok;
        let resource_ok = resource.is_ok() && measured.is_ok();
        writeln!(
            evidence,
            "final semantic={} resource={} cleanup={}",
            if semantic_ok { "passed" } else { "failed" },
            if resource_ok { "passed" } else { "failed" },
            if cleanup_ok { "passed" } else { "failed" }
        )?;
        evidence.sync_all()?;
        println!(
            "ocr-text-watch-windows: semantic={} resource={} cleanup={}",
            if semantic_ok { "passed" } else { "failed" },
            if resource_ok { "passed" } else { "failed" },
            if cleanup_ok { "passed" } else { "failed" }
        );
        Ok(semantic_ok && resource_ok && cleanup_ok)
    }
}
