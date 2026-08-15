//! Benchmark-only controller for the separately linked private macOS fixture.
//!
//! This module is compiled into benchmark artifacts only. It owns one fixture
//! child, one bounded Unix-domain control connection, and one outstanding
//! command at a time. A fixture acknowledgement proves only that the private
//! command ran; callers must establish capture progress or product input
//! delivery through independent oracles.

use std::collections::VecDeque;
use std::fmt;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::macos_fixture_control::{
    AuthenticatedFixtureProcess, FixtureSocketDirectory, authenticate_fixture_peer,
    next_fixture_run_nonce,
};
use crate::macos_fixture_protocol::{
    self as protocol, EVENT_FLAGS_CHANGED, EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_POINTER_MOVE,
    EVENT_POINTER_PRESS, EVENT_POINTER_RELEASE, EVENT_POINTER_SCROLL, EventSummary, EventTotals,
    FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureMode, FixtureRenderer,
    MAX_CONTROL_LINE_BYTES, MAX_RECORDED_EVENTS, fixture_ready_facts, format_command_line,
    parse_command_result_line, parse_event_line_for_run,
};
use mado_pilot::CancellationToken;

const MAX_OUTPUT_LINE_BYTES: usize = 1_024;
const READER_QUEUE_CAPACITY: usize = MAX_RECORDED_EVENTS + 16;
const WAIT_SLICE: Duration = Duration::from_millis(25);
const DROP_WAIT: Duration = Duration::from_secs(1);
const GRACEFUL_CLOSE_WAIT: Duration = Duration::from_millis(100);
const EVENT_QUIET_WAIT: Duration = Duration::from_millis(100);

/// How the independently linked fixture renders and whether product input also
/// drives its legacy interactive animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Static AppKit background, changed only by private commands.
    Static,
    /// The retained interactive `System` profile's input-driven animation.
    AnimateOnInput,
    /// The retained interactive `System` profile's animation and resize mode.
    AnimateAndResizeOnInput,
    /// Independently qualified dynamically loaded OpenGL renderer.
    GameLike,
}

impl LaunchMode {
    fn argument(self) -> Option<&'static str> {
        match self {
            Self::Static => None,
            Self::AnimateOnInput => Some("--animate-on-input"),
            Self::AnimateAndResizeOnInput => Some("--animate-and-resize-on-input"),
            Self::GameLike => Some("--game-like"),
        }
    }

    fn expected_facts(self) -> (FixtureMode, FixtureRenderer) {
        match self {
            Self::GameLike => (FixtureMode::GameLike, FixtureRenderer::OpenGl),
            Self::Static | Self::AnimateOnInput | Self::AnimateAndResizeOnInput => {
                (FixtureMode::Default, FixtureRenderer::AppKitBackground)
            }
        }
    }

    /// Stable, non-sensitive profile fact emitted in benchmark metadata.
    pub const fn fact(self) -> &'static str {
        match self {
            Self::GameLike => "mode=game-like renderer=opengl",
            Self::Static | Self::AnimateOnInput | Self::AnimateAndResizeOnInput => {
                "mode=default renderer=appkit-background"
            }
        }
    }
}

/// One correlated private command acknowledgement and its transport latency.
///
/// The result is never a product receipt or visual outcome.
#[derive(Debug, Clone, Copy)]
pub struct CommandAcknowledgement {
    result: FixtureCommandResult,
    elapsed: Duration,
}

impl CommandAcknowledgement {
    /// The exact private protocol result.
    pub const fn result(self) -> FixtureCommandResult {
        self.result
    }

    /// Time from the first command write through its correlated result line.
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

/// Observation made by a helper that cancels only after one expected fixture
/// event has arrived.
#[derive(Debug, Clone, Copy)]
pub struct CancellationObservation {
    summary: Option<EventSummary>,
    cancelled_at: Option<Instant>,
}

impl CancellationObservation {
    /// The event summary that triggered cancellation, if the exact event arrived.
    pub const fn summary(self) -> Option<EventSummary> {
        self.summary
    }

    /// The monotonic instant immediately before cancellation was signalled.
    pub const fn cancelled_at(self) -> Option<Instant> {
        self.cancelled_at
    }
}

enum ReaderMessage {
    Line(String),
    Oversized,
    Failed,
}

/// One owned fixture application and its bounded command/event channel.
pub struct FixtureController {
    launcher: Child,
    application: AuthenticatedFixtureProcess,
    input: Option<UnixStream>,
    lines: Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader: Option<thread::JoinHandle<()>>,
    reader_failed: Arc<AtomicBool>,
    pending_events: VecDeque<EventSummary>,
    run_nonce: u64,
    next_nonce: u64,
    launch_mode: LaunchMode,
    stopped: bool,
    finish_result: Option<bool>,
}

impl fmt::Debug for FixtureController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixtureController")
            .field("launch_mode", &self.launch_mode)
            .field("pending_events", &self.pending_events.len())
            .field("run_nonce", &self.run_nonce)
            .field("next_nonce", &self.next_nonce)
            .field("stopped", &self.stopped)
            .finish_non_exhaustive()
    }
}

impl FixtureController {
    /// Launches one signed app bundle derived from `executable` and waits for its
    /// exact protocol-v6 ready facts.
    pub fn start(
        executable: &Path,
        launch_mode: LaunchMode,
        wait: Duration,
    ) -> Result<Self, String> {
        let executable = executable
            .canonicalize()
            .map_err(|_| "the fixture executable cannot be canonicalized".to_owned())?;
        let bundle = fixture_bundle(&executable).ok_or_else(|| {
            "the fixture executable is not inside a .app/Contents/MacOS bundle".to_owned()
        })?;
        let socket_directory = FixtureSocketDirectory::new()?;
        let socket_path = socket_directory.socket_path();
        let run_nonce = next_fixture_run_nonce()?;
        let listener = UnixListener::bind(&socket_path)
            .map_err(|_| "the fixture control listener could not bind".to_owned())?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "the fixture control listener could not be bounded".to_owned())?;

        let mut launch = Command::new("/usr/bin/open");
        launch
            .arg("-n")
            .arg("-W")
            .arg(&bundle)
            .arg("--args")
            .arg("--control-socket")
            .arg(&socket_path)
            .arg("--run-nonce")
            .arg(run_nonce.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        if let Some(argument) = launch_mode.argument() {
            launch.arg(argument);
        }
        let child = launch
            .spawn()
            .map_err(|_| "the fixture application launcher could not start".to_owned())?;
        let mut launch_guard = LaunchGuard::new(child);
        let deadline = Instant::now() + wait;
        let (stream, application) = loop {
            match listener.accept() {
                Ok((stream, _address)) => {
                    if let Some(application) = authenticate_fixture_peer(&stream, &executable) {
                        launch_guard.application = Some(application);
                        break (stream, application);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err("the fixture control listener failed".to_owned()),
            }
            if launch_guard.exited()? {
                return Err("the fixture application exited before connecting".to_owned());
            }
            if Instant::now() >= deadline {
                return Err(
                    "the fixture application did not connect before the deadline".to_owned(),
                );
            }
            thread::sleep(WAIT_SLICE);
        };
        let process_id = application.process_id();
        stream
            .set_nonblocking(false)
            .map_err(|_| "the fixture control connection could not become blocking".to_owned())?;
        drop(listener);
        drop(socket_directory);
        let input = stream
            .try_clone()
            .map_err(|_| "the fixture control connection could not be cloned".to_owned())?;
        let (sender, receiver) = mpsc::sync_channel(READER_QUEUE_CAPACITY);
        let reader_failed = Arc::new(AtomicBool::new(false));
        let reader_failure = Arc::clone(&reader_failed);
        let reader = thread::Builder::new()
            .name("mado-pilot-benchmark-fixture-reader".to_owned())
            .spawn(move || read_bounded_lines(stream, &sender, &reader_failure))
            .map_err(|_| "the fixture output reader could not start".to_owned())?;
        let lines = Arc::new(Mutex::new(receiver));
        let ready = wait_for_ready(&lines, &reader_failed, deadline)?;
        let facts = fixture_ready_facts(&ready, process_id)
            .ok_or_else(|| "the fixture ready record did not match protocol v6".to_owned())?;
        let expected = launch_mode.expected_facts();
        if !facts.execution_context_is_approved()
            || facts.run_nonce() != run_nonce
            || facts.mode() != expected.0
            || facts.renderer() != expected.1
            || facts.window_number() == 0
        {
            return Err(
                "the fixture ready facts did not match the requested approved run and mode"
                    .to_owned(),
            );
        }

        let (launcher, application) = launch_guard.take();
        Ok(Self {
            launcher,
            application,
            input: Some(input),
            lines,
            reader: Some(reader),
            reader_failed,
            pending_events: VecDeque::new(),
            run_nonce,
            next_nonce: 1,
            launch_mode,
            stopped: false,
            finish_result: None,
        })
    }

    /// Owned application process identity used only for fail-closed fixture selection.
    pub const fn process_id(&self) -> u32 {
        self.application.process_id()
    }

    /// Returns the still-live audit-token identity only while the authenticated
    /// control connection remains usable.
    pub fn authenticated_process(&self) -> Option<AuthenticatedFixtureProcess> {
        if self.stopped
            || self.input.is_none()
            || self.reader_failed.load(Ordering::Acquire)
            || !self
                .application
                .matches_live_owner(i64::from(self.application.process_id()))
        {
            return None;
        }
        Some(self.application)
    }

    /// The explicit renderer/mode fact validated from the ready record.
    pub const fn launch_mode(&self) -> LaunchMode {
        self.launch_mode
    }

    /// Sends exactly one private command and waits for its matching result.
    pub fn command(
        &mut self,
        kind: FixtureCommandKind,
        wait: Duration,
    ) -> Result<CommandAcknowledgement, String> {
        self.send_command(kind, 0, wait)
    }

    fn send_command(
        &mut self,
        kind: FixtureCommandKind,
        event_payload_tag: u64,
        wait: Duration,
    ) -> Result<CommandAcknowledgement, String> {
        if self.stopped {
            return Err("the fixture controller is already stopped".to_owned());
        }
        let nonce = self.next_nonce;
        self.next_nonce = self
            .next_nonce
            .checked_add(1)
            .ok_or_else(|| "the fixture command identity is exhausted".to_owned())?;

        let encoded = format_command_line(FixtureCommand {
            run_nonce: self.run_nonce,
            nonce,
            event_payload_tag,
            kind,
        });
        if encoded.len().saturating_add(1) > MAX_CONTROL_LINE_BYTES {
            return Err("the encoded fixture command exceeds its fixed line bound".to_owned());
        }
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| "the fixture command channel is closed".to_owned())?;
        let started = Instant::now();
        writeln!(input, "{encoded}")
            .and_then(|()| input.flush())
            .map_err(|_| "the fixture command could not be written".to_owned())?;
        let deadline = started + wait;
        loop {
            let message = recv_message(&self.lines, &self.reader_failed, deadline)?;
            match message {
                ReaderMessage::Line(line) => {
                    if let Some(event) = parse_event_line_for_run(&line, self.run_nonce) {
                        if self.pending_events.len() == MAX_RECORDED_EVENTS {
                            return Err("the bounded fixture event buffer is full".to_owned());
                        }
                        self.pending_events.push_back(event);
                    } else if let Some(result) = parse_command_result_line(&line) {
                        if result.run_nonce != self.run_nonce || result.nonce != nonce {
                            return Err(
                                "the fixture returned an out-of-run or out-of-order command result"
                                    .to_owned(),
                            );
                        }
                        return Ok(CommandAcknowledgement {
                            result,
                            elapsed: started.elapsed(),
                        });
                    } else if line.starts_with("fixture-command-") {
                        return Err("the fixture rejected or malformed a command".to_owned());
                    } else {
                        return Err("the fixture emitted an unexpected protocol record".to_owned());
                    }
                }
                ReaderMessage::Oversized => {
                    return Err("the fixture emitted an oversized protocol line".to_owned());
                }
                ReaderMessage::Failed => {
                    return Err("the fixture output reader failed".to_owned());
                }
            }
        }
    }
    /// Resets the fixture's bounded event counters and refuses queued prior-run output.
    pub fn reset_events(&mut self, event_payload_tag: u64, wait: Duration) -> bool {
        if !self.pending_events.is_empty() {
            return false;
        }
        self.send_command(FixtureCommandKind::ResetEvents, event_payload_tag, wait)
            .is_ok_and(|ack| {
                ack.result.status == 0
                    && ack.result.events == EventTotals::default()
                    && self.pending_events.is_empty()
            })
    }

    /// Reads one exact event row and verifies the fixture's independent totals.
    pub fn events_are_exact(&mut self, expected: &[EventSummary], wait: Duration) -> bool {
        self.remaining_events_are_exact(expected, expected, wait)
    }

    /// Reads an exact suffix after an exclusive observer consumed a known prefix,
    /// then verifies the fixture's cumulative independent totals for the full row.
    pub fn remaining_events_are_exact(
        &mut self,
        expected_remaining: &[EventSummary],
        expected_total: &[EventSummary],
        wait: Duration,
    ) -> bool {
        let deadline = Instant::now() + wait;
        let observed = self.event_summaries(
            expected_remaining.len(),
            deadline.saturating_duration_since(Instant::now()),
        );
        let Some(totals) = event_totals(expected_total) else {
            return false;
        };
        let expected_correlation = expected_total.first().map_or(0, |event| event.correlation);
        let correlation_is_exact = expected_total
            .iter()
            .all(|event| event.correlation == expected_correlation);
        let observed_report = self
            .command(
                FixtureCommandKind::ReadEvents,
                deadline.saturating_duration_since(Instant::now()),
            )
            .ok()
            .filter(|ack| ack.result.status == 0)
            .map(|ack| ack.result);
        let report_is_exact = observed_report.is_some_and(|report| {
            report.events == totals
                && (expected_correlation == 0
                    || (correlation_is_exact
                        && report.event_correlation == expected_correlation
                        && report.event_payload_matches))
        });
        let quiet = Instant::now()
            .checked_add(EVENT_QUIET_WAIT)
            .filter(|quiet_deadline| *quiet_deadline <= deadline)
            .is_some_and(|quiet_deadline| self.output_remains_quiet(quiet_deadline));
        observed == expected_remaining && report_is_exact && self.pending_events.is_empty() && quiet
    }

    fn output_remains_quiet(&self, deadline: Instant) -> bool {
        while Instant::now() < deadline {
            if self.reader_failed.load(Ordering::Acquire) {
                return false;
            }
            let received = self
                .lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(WAIT_SLICE),
                );
            match received {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        !self.reader_failed.load(Ordering::Acquire)
    }

    /// Returns exactly `count` bounded event summaries or fewer on timeout/failure.
    pub fn event_summaries(&mut self, count: usize, wait: Duration) -> Vec<EventSummary> {
        let mut events = Vec::with_capacity(count.min(MAX_RECORDED_EVENTS));
        while events.len() < count {
            if let Some(event) = self.pending_events.pop_front() {
                events.push(event);
            } else {
                break;
            }
        }
        let deadline = Instant::now() + wait;
        while events.len() < count && Instant::now() < deadline {
            let Ok(message) = recv_message(&self.lines, &self.reader_failed, deadline) else {
                break;
            };
            match message {
                ReaderMessage::Line(line) => {
                    if let Some(event) = parse_event_line_for_run(&line, self.run_nonce) {
                        events.push(event);
                    } else {
                        self.reader_failed.store(true, Ordering::Release);
                        break;
                    }
                }
                ReaderMessage::Oversized | ReaderMessage::Failed => {
                    self.reader_failed.store(true, Ordering::Release);
                    break;
                }
            }
        }
        events
    }

    /// Starts one helper that cancels only after the exact event arrives.
    ///
    /// The receiver is exclusive while the helper runs. Callers must join it
    /// before reading another acknowledgement or event.
    pub fn cancel_after_event(
        &mut self,
        expected: EventSummary,
        cancellation: CancellationToken,
        wait: Duration,
    ) -> Result<thread::JoinHandle<CancellationObservation>, String> {
        if !self.pending_events.is_empty() {
            return Err("stale fixture events precede the cleanup sample".to_owned());
        }
        if self.reader_failed.load(Ordering::Acquire) {
            return Err("the fixture output protocol already failed".to_owned());
        }
        {
            let receiver = self
                .lines
                .lock()
                .map_err(|_| "the fixture output receiver is poisoned".to_owned())?;
            match receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("the fixture output channel is disconnected".to_owned());
                }
                Ok(_) => return Err("queued fixture output precedes the cleanup sample".to_owned()),
            }
        }
        let lines = Arc::clone(&self.lines);
        let run_nonce = self.run_nonce;
        let reader_failed = Arc::clone(&self.reader_failed);
        thread::Builder::new()
            .name("mado-pilot-benchmark-cleanup-trigger".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + wait;
                let Ok(message) = recv_message(&lines, &reader_failed, deadline) else {
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                let ReaderMessage::Line(line) = message else {
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                let Some(summary) = parse_event_line_for_run(&line, run_nonce) else {
                    reader_failed.store(true, Ordering::Release);
                    return CancellationObservation {
                        summary: None,
                        cancelled_at: None,
                    };
                };
                if summary != expected {
                    return CancellationObservation {
                        summary: Some(summary),
                        cancelled_at: None,
                    };
                }
                let cancelled_at = Instant::now();
                cancellation.cancel();
                CancellationObservation {
                    summary: Some(summary),
                    cancelled_at: Some(cancelled_at),
                }
            })
            .map_err(|_| "the cleanup observation helper could not start".to_owned())
    }
    /// Stops the private fixture, terminates its authenticated application when
    /// needed, reaps the owned launcher, and verifies no event remained. Idempotent.
    pub fn finish(&mut self, wait: Duration) -> bool {
        if let Some(result) = self.finish_result {
            return result;
        }
        let deadline = Instant::now() + wait;
        let acknowledged = self
            .command(
                FixtureCommandKind::Stop,
                deadline.saturating_duration_since(Instant::now()),
            )
            .is_ok_and(|ack| ack.result.status == 0);
        self.shutdown_input();
        let graceful_deadline = deadline.min(Instant::now() + GRACEFUL_CLOSE_WAIT);
        let mut reaped = wait_for_exit(&mut self.launcher, graceful_deadline);
        if !reaped {
            reaped = terminate_authenticated_application(
                &mut self.application,
                &mut self.launcher,
                deadline,
            );
        }
        let bounded = reaped && Instant::now() <= deadline;
        if !reaped {
            let _killed = self.launcher.kill();
            reaped = wait_for_exit(&mut self.launcher, Instant::now() + DROP_WAIT);
        }
        self.stopped = reaped;

        let output_clean = finish_reader_output_is_clean(
            self.reader.take(),
            &self.lines,
            &self.reader_failed,
            self.pending_events.is_empty(),
            deadline,
        );
        let result = acknowledged && reaped && bounded && output_clean;
        self.finish_result = Some(result);
        result
    }

    fn shutdown_input(&mut self) {
        if let Some(input) = self.input.take() {
            let _shutdown = input.shutdown(Shutdown::Both);
        }
    }

    fn terminate_owned(&mut self) {
        self.shutdown_input();
        let deadline = Instant::now() + DROP_WAIT;
        let mut reaped = terminate_authenticated_application(
            &mut self.application,
            &mut self.launcher,
            deadline,
        );
        if !reaped {
            let _killed = self.launcher.kill();
            reaped = wait_for_exit(&mut self.launcher, Instant::now() + DROP_WAIT);
        }
        self.stopped = reaped;
        self.finish_result = Some(false);
        let _output_clean = finish_reader_output_is_clean(
            self.reader.take(),
            &self.lines,
            &self.reader_failed,
            self.pending_events.is_empty(),
            deadline,
        );
    }
}
fn event_totals(events: &[EventSummary]) -> Option<EventTotals> {
    let mut totals = EventTotals::default();
    for event in events {
        let count = match event.kind {
            EVENT_POINTER_MOVE => &mut totals.pointer_moves,
            EVENT_POINTER_PRESS => &mut totals.pointer_presses,
            EVENT_POINTER_RELEASE => &mut totals.pointer_releases,
            EVENT_POINTER_SCROLL => &mut totals.pointer_scrolls,
            EVENT_KEY_DOWN => &mut totals.key_downs,
            EVENT_KEY_UP => &mut totals.key_ups,
            EVENT_FLAGS_CHANGED => &mut totals.flags_changed,
            _ => return None,
        };
        *count = count.checked_add(1)?;
        totals.text_units = totals.text_units.checked_add(u64::from(event.text_units))?;
    }
    Some(totals)
}

impl Drop for FixtureController {
    fn drop(&mut self) {
        if !self.stopped {
            self.terminate_owned();
        }
    }
}

fn finish_reader_output_is_clean(
    reader: Option<thread::JoinHandle<()>>,
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    pending_events_empty: bool,
    deadline: Instant,
) -> bool {
    let mut output_clean = pending_events_empty && !reader_failed.load(Ordering::Acquire);
    if let Some(reader) = reader {
        while !reader.is_finished() && Instant::now() < deadline {
            let received = lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(WAIT_SLICE),
                );
            match received {
                Ok(_message) => output_clean = false,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if reader.is_finished() {
            output_clean &= reader.join().is_ok();
        } else {
            output_clean = false;
        }
    }
    loop {
        let received = lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_recv();
        match received {
            Ok(_message) => output_clean = false,
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
        }
    }
    output_clean && !reader_failed.load(Ordering::Acquire)
}

fn wait_for_ready(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    deadline: Instant,
) -> Result<String, String> {
    match recv_message(lines, reader_failed, deadline)? {
        ReaderMessage::Line(line) if line.starts_with("fixture-ready ") => Ok(line),
        ReaderMessage::Line(_other) => {
            Err("the fixture emitted a protocol record before readiness".to_owned())
        }
        ReaderMessage::Oversized => Err("the fixture emitted an oversized ready record".to_owned()),
        ReaderMessage::Failed => Err("the fixture output reader failed".to_owned()),
    }
}

fn recv_message(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader_failed: &AtomicBool,
    deadline: Instant,
) -> Result<ReaderMessage, String> {
    loop {
        if reader_failed.load(Ordering::Acquire) {
            return Err("the fixture output protocol failed".to_owned());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("the fixture protocol operation exceeded its deadline".to_owned());
        }
        let received = lines
            .lock()
            .map_err(|_| "the fixture output receiver is poisoned".to_owned())?
            .recv_timeout(remaining.min(WAIT_SLICE));
        match received {
            Ok(message) => return Ok(message),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err("the fixture protocol operation exceeded its deadline".to_owned());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the fixture output channel disconnected".to_owned());
            }
        }
    }
}

fn read_bounded_lines(
    mut stream: UnixStream,
    sender: &mpsc::SyncSender<ReaderMessage>,
    reader_failed: &AtomicBool,
) {
    let mut line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
    let mut byte = [0u8; 1];
    let mut overflow = false;
    loop {
        match stream.read(&mut byte) {
            Ok(0) => {
                if overflow || !line.is_empty() {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Failed);
                }
                return;
            }
            Ok(_) if byte[0] == b'\n' => {
                if overflow {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Oversized);
                    return;
                }
                let Ok(decoded) = String::from_utf8(std::mem::take(&mut line)) else {
                    reader_failed.store(true, Ordering::Release);
                    let _sent = sender.try_send(ReaderMessage::Failed);
                    return;
                };
                if sender.try_send(ReaderMessage::Line(decoded)).is_err() {
                    reader_failed.store(true, Ordering::Release);
                    return;
                }
                line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
            }
            Ok(_) if !overflow && line.len() < MAX_OUTPUT_LINE_BYTES.saturating_sub(1) => {
                line.push(byte[0]);
            }
            Ok(_) => overflow = true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                reader_failed.store(true, Ordering::Release);
                let _sent = sender.try_send(ReaderMessage::Failed);
                return;
            }
        }
    }
}

fn fixture_bundle(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

fn wait_for_exit(child: &mut Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => return false,
        }
    }
}

fn terminate_authenticated_application(
    application: &mut AuthenticatedFixtureProcess,
    launcher: &mut Child,
    deadline: Instant,
) -> bool {
    let _terminated = application.terminate();
    let term_deadline = deadline.min(Instant::now() + GRACEFUL_CLOSE_WAIT);
    if wait_for_exit(launcher, term_deadline) {
        return true;
    }
    let _killed = application.kill();
    wait_for_exit(launcher, deadline)
}

struct LaunchGuard {
    child: Option<Child>,
    application: Option<AuthenticatedFixtureProcess>,
}

impl LaunchGuard {
    fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            application: None,
        }
    }

    fn exited(&mut self) -> Result<bool, String> {
        self.child
            .as_mut()
            .expect("the guarded launcher exists")
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|_| "the fixture launcher state could not be read".to_owned())
    }

    fn take(mut self) -> (Child, AuthenticatedFixtureProcess) {
        let application = self
            .application
            .take()
            .expect("the authenticated fixture process exists");
        let child = self.child.take().expect("the guarded launcher exists");
        (child, application)
    }
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        let deadline = Instant::now() + DROP_WAIT;
        let mut reaped = false;
        if let (Some(application), Some(child)) = (self.application.as_mut(), self.child.as_mut()) {
            reaped = terminate_authenticated_application(application, child, deadline);
        }
        if let Some(child) = self.child.as_mut()
            && !reaped
        {
            let _killed = child.kill();
            let _reaped = wait_for_exit(child, Instant::now() + DROP_WAIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_OUTPUT_LINE_BYTES, ReaderMessage, finish_reader_output_is_clean, fixture_bundle,
        read_bounded_lines,
    };
    use crate::macos_fixture_protocol::{EVENT_KEY_DOWN, EventSummary, format_event_line};
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn fixture_bundle_derivation_accepts_only_the_expected_layout() {
        assert_eq!(
            fixture_bundle(Path::new(
                "/tmp/MadoPilotFixture.app/Contents/MacOS/mado-pilot-macos-input-fixture"
            )),
            Some(PathBuf::from("/tmp/MadoPilotFixture.app"))
        );
        assert_eq!(
            fixture_bundle(Path::new("/tmp/mado-pilot-macos-input-fixture")),
            None
        );
    }

    #[test]
    fn finish_drains_an_event_enqueued_before_reader_eof() {
        let run_nonce = 91;
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            sender
                .send(ReaderMessage::Line("fixture-log".to_owned()))
                .expect("the first line is queued");
            sender
                .send(ReaderMessage::Line(format_event_line(
                    run_nonce,
                    EventSummary {
                        kind: EVENT_KEY_DOWN,
                        text_units: 0,
                        correlation: 0,
                    },
                )))
                .expect("the trailing event is queued");
        });
        let lines = Arc::new(Mutex::new(receiver));

        assert!(!finish_reader_output_is_clean(
            Some(reader),
            &lines,
            &AtomicBool::new(false),
            true,
            Instant::now() + Duration::from_secs(1),
        ));
    }

    #[test]
    fn output_lines_are_bounded_including_the_newline() {
        let (reader, mut writer) = UnixStream::pair().expect("private socket pair opens");
        let (sender, receiver) = mpsc::sync_channel(2);
        let reader_failed = Arc::new(AtomicBool::new(false));
        let task_failed = Arc::clone(&reader_failed);
        let task = thread::spawn(move || read_bounded_lines(reader, &sender, &task_failed));
        writeln!(writer, "{}", "x".repeat(MAX_OUTPUT_LINE_BYTES - 1))
            .expect("the maximum line is written");
        writeln!(writer, "{}", "x".repeat(MAX_OUTPUT_LINE_BYTES))
            .expect("the oversized line is written");
        drop(writer);

        assert!(matches!(
            receiver.recv().expect("the bounded line arrives"),
            ReaderMessage::Line(line) if line.len() == MAX_OUTPUT_LINE_BYTES - 1
        ));
        assert!(matches!(
            receiver.recv().expect("the oversized marker arrives"),
            ReaderMessage::Oversized
        ));
        task.join().expect("the bounded reader exits");
    }
}
