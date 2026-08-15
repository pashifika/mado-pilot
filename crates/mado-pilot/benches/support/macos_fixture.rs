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
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::macos_fixture_protocol::{
    self as protocol, EventSummary, FixtureCommand, FixtureCommandKind, FixtureCommandResult,
    FixtureMode, FixtureRenderer, MAX_CONTROL_LINE_BYTES, MAX_RECORDED_EVENTS, fixture_ready_facts,
    format_command_line, parse_command_result_line, parse_event_line_for_run,
};
use mado_pilot::CancellationToken;

const MAX_OUTPUT_LINE_BYTES: usize = 1_024;
const READER_QUEUE_CAPACITY: usize = MAX_RECORDED_EVENTS + 16;
const WAIT_SLICE: Duration = Duration::from_millis(25);
const DROP_WAIT: Duration = Duration::from_secs(1);

static SOCKET_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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
    application_pid: u32,
    input: Option<UnixStream>,
    lines: Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    reader: Option<thread::JoinHandle<()>>,
    pending_events: VecDeque<EventSummary>,
    run_nonce: u64,
    next_nonce: u64,
    launch_mode: LaunchMode,
    stopped: bool,
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
    /// exact protocol-v5 ready facts.
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
        let socket = SocketPath::new();
        let run_nonce = next_run_nonce();
        let listener = UnixListener::bind(socket.path())
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
            .arg(socket.path())
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
        let stream = loop {
            match listener.accept() {
                Ok((stream, _address)) => break stream,
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
        stream
            .set_nonblocking(false)
            .map_err(|_| "the fixture control connection could not become blocking".to_owned())?;
        let _removed = std::fs::remove_file(socket.path());
        let input = stream
            .try_clone()
            .map_err(|_| "the fixture control connection could not be cloned".to_owned())?;
        let (sender, receiver) = mpsc::sync_channel(READER_QUEUE_CAPACITY);
        let reader = thread::Builder::new()
            .name("mado-pilot-benchmark-fixture-reader".to_owned())
            .spawn(move || read_bounded_lines(stream, &sender))
            .map_err(|_| "the fixture output reader could not start".to_owned())?;
        let lines = Arc::new(Mutex::new(receiver));
        let ready = wait_for_ready(&lines, deadline)?;
        let process_id = ready_process_id(&ready).ok_or_else(|| {
            "the fixture ready record omitted its owned process identity".to_owned()
        })?;
        launch_guard.application_pid = Some(process_id);
        let facts = fixture_ready_facts(&ready, process_id)
            .ok_or_else(|| "the fixture ready record did not match protocol v5".to_owned())?;
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

        Ok(Self {
            launcher: launch_guard.take(),
            application_pid: process_id,
            input: Some(input),
            lines,
            reader: Some(reader),
            pending_events: VecDeque::new(),
            run_nonce,
            next_nonce: 1,
            launch_mode,
            stopped: false,
        })
    }

    /// Owned application process identity used only for fail-closed fixture selection.
    pub const fn process_id(&self) -> u32 {
        self.application_pid
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
            let message = recv_message(&self.lines, deadline)?;
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
            let Ok(message) = recv_message(&self.lines, deadline) else {
                break;
            };
            match message {
                ReaderMessage::Line(line) => {
                    if let Some(event) = parse_event_line_for_run(&line, self.run_nonce) {
                        events.push(event);
                    } else if line.starts_with("fixture-command-") {
                        break;
                    }
                }
                ReaderMessage::Oversized | ReaderMessage::Failed => break,
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
        thread::Builder::new()
            .name("mado-pilot-benchmark-cleanup-trigger".to_owned())
            .spawn(move || {
                let deadline = Instant::now() + wait;
                while Instant::now() < deadline {
                    let Ok(message) = recv_message(&lines, deadline) else {
                        break;
                    };
                    let ReaderMessage::Line(line) = message else {
                        break;
                    };
                    let Some(summary) = parse_event_line_for_run(&line, run_nonce) else {
                        continue;
                    };
                    if summary != expected {
                        return CancellationObservation {
                            summary: Some(summary),
                            cancelled_at: None,
                        };
                    }
                    let cancelled_at = Instant::now();
                    cancellation.cancel();
                    return CancellationObservation {
                        summary: Some(summary),
                        cancelled_at: Some(cancelled_at),
                    };
                }
                CancellationObservation {
                    summary: None,
                    cancelled_at: None,
                }
            })
            .map_err(|_| "the cleanup observation helper could not start".to_owned())
    }
    /// Stops the private fixture, reaps only the owned launcher, and verifies no
    /// unconsumed event remained. Idempotent.
    pub fn finish(&mut self, wait: Duration) -> bool {
        if self.stopped {
            return true;
        }
        let deadline = Instant::now() + wait;
        let acknowledged = self
            .command(FixtureCommandKind::Stop, wait)
            .is_ok_and(|ack| ack.result.status == 0);
        self.input = None;
        self.stopped = true;
        let reaped = wait_for_exit(&mut self.launcher, deadline);
        if !reaped {
            self.terminate_owned();
            return false;
        }
        let mut no_trailing_event = self.pending_events.is_empty();
        while let Ok(message) = self
            .lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_recv()
        {
            if let ReaderMessage::Line(line) = message
                && parse_event_line_for_run(&line, self.run_nonce).is_some()
            {
                no_trailing_event = false;
            }
        }
        let reader_ok = self
            .reader
            .take()
            .is_none_or(|reader| reader.join().is_ok());
        acknowledged && no_trailing_event && reader_ok
    }

    fn terminate_owned(&mut self) {
        self.input = None;
        terminate_application(self.application_pid);
        let deadline = Instant::now() + DROP_WAIT;
        if !wait_for_exit(&mut self.launcher, deadline) {
            let _killed = self.launcher.kill();
            let _reaped = self.launcher.wait();
        }
        self.stopped = true;
        if let Some(reader) = self.reader.take() {
            let _joined = reader.join();
        }
    }
}

impl Drop for FixtureController {
    fn drop(&mut self) {
        if !self.stopped {
            let _finished = self.finish(DROP_WAIT);
        }
    }
}

fn wait_for_ready(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    deadline: Instant,
) -> Result<String, String> {
    loop {
        match recv_message(lines, deadline)? {
            ReaderMessage::Line(line) if line.starts_with("fixture-ready ") => return Ok(line),
            ReaderMessage::Line(_other) => {}
            ReaderMessage::Oversized => {
                return Err("the fixture emitted an oversized ready record".to_owned());
            }
            ReaderMessage::Failed => return Err("the fixture output reader failed".to_owned()),
        }
    }
}

fn recv_message(
    lines: &Arc<Mutex<mpsc::Receiver<ReaderMessage>>>,
    deadline: Instant,
) -> Result<ReaderMessage, String> {
    loop {
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

fn read_bounded_lines(mut stream: UnixStream, sender: &mpsc::SyncSender<ReaderMessage>) {
    let mut line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
    let mut byte = [0u8; 1];
    let mut overflow = false;
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return,
            Ok(_) if byte[0] == b'\n' => {
                let message = if overflow {
                    ReaderMessage::Oversized
                } else {
                    match String::from_utf8(std::mem::take(&mut line)) {
                        Ok(line) => ReaderMessage::Line(line),
                        Err(_) => ReaderMessage::Failed,
                    }
                };
                line = Vec::with_capacity(MAX_OUTPUT_LINE_BYTES);
                overflow = false;
                if sender.send(message).is_err() {
                    return;
                }
            }
            Ok(_) if !overflow && line.len() < MAX_OUTPUT_LINE_BYTES.saturating_sub(1) => {
                line.push(byte[0]);
            }
            Ok(_) => overflow = true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                let _sent = sender.send(ReaderMessage::Failed);
                return;
            }
        }
    }
}

fn next_run_nonce() -> u64 {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        });
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    (timestamp ^ (u64::from(std::process::id()) << 32) ^ sequence).max(1)
}

fn ready_process_id(line: &str) -> Option<u32> {
    let (_prefix, remainder) = line.split_once(" pid=")?;
    let (process_id, _suffix) = remainder.split_once(' ')?;
    let process_id = process_id.parse().ok()?;
    line.starts_with(&format!(
        "fixture-ready title={} ",
        protocol::fixture_title(process_id)
    ))
    .then_some(process_id)
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
            Ok(Some(_status)) => {
                let _reaped = child.wait();
                return true;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => return false,
        }
    }
}

fn terminate_application(process_id: u32) {
    let _terminated = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(process_id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

struct SocketPath(PathBuf);

impl SocketPath {
    fn new() -> Self {
        let sequence = SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self(PathBuf::from(format!(
            "/tmp/mado-pilot-bench-{}-{sequence}.sock",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SocketPath {
    fn drop(&mut self) {
        let _removed = std::fs::remove_file(&self.0);
    }
}

struct LaunchGuard {
    child: Option<Child>,
    application_pid: Option<u32>,
}

impl LaunchGuard {
    fn new(child: Child) -> Self {
        Self {
            child: Some(child),
            application_pid: None,
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

    fn take(mut self) -> Child {
        self.application_pid = None;
        self.child.take().expect("the guarded launcher exists")
    }
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        if let Some(process_id) = self.application_pid {
            terminate_application(process_id);
        }
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().ok().flatten().is_none() {
                let _killed = child.kill();
            }
            let _reaped = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_OUTPUT_LINE_BYTES, ReaderMessage, fixture_bundle, read_bounded_lines, ready_process_id,
    };
    use crate::macos_fixture_protocol::fixture_title;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;

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
    fn ready_process_identity_requires_the_matching_process_qualified_title() {
        let process_id = 42;
        let line = format!(
            "fixture-ready title={} pid={process_id} window=7 run=91 control-version=5 \
             mode=default renderer=appkit-background",
            fixture_title(process_id)
        );
        assert_eq!(ready_process_id(&line), Some(process_id));
        assert_eq!(
            ready_process_id(&line.replace(&fixture_title(process_id), "unrelated")),
            None
        );
    }

    #[test]
    fn output_lines_are_bounded_including_the_newline() {
        let (reader, mut writer) = UnixStream::pair().expect("private socket pair opens");
        let (sender, receiver) = mpsc::sync_channel(2);
        let task = thread::spawn(move || read_bounded_lines(reader, &sender));
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
