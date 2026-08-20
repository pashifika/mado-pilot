//! Dedicated, deterministic target for interactive macOS input verification.
//!
//! The fixture shows one window filled with one approved deterministic colour,
//! publishes an exact process-qualified title for fail-closed selection, and
//! prints a bounded summary of the events that reach it. It retains no characters:
//! a key event is reported as its kind and its UTF-16 unit count.
//!
//! The default path remains an AppKit background colour. The explicit game-like
//! mode uses a separately loaded OpenGL-backed content view. Neither is an input
//! channel: qualification submits only through the production Adapter's explicit
//! process-directed route, whose truthful evidence is native invocation.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("mado-pilot-macos-input-fixture requires macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
use mado_pilot_platform_macos::fixture_protocol;

#[cfg(target_os = "macos")]
fn main() {
    let mut mode = None;
    let mut control_socket = None;
    let mut run_nonce = None;
    let mut report_execution_context = false;
    let mut inactive = false;
    let mut independent_visual_stimulus = false;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        let parsed_mode = if argument == std::ffi::OsStr::new("--replace-window-after-ready") {
            Some(fixture::Mode::Replace)
        } else if argument == std::ffi::OsStr::new("--animate-on-input") {
            Some(fixture::Mode::AnimateOnInput)
        } else if argument == std::ffi::OsStr::new("--resize-on-input") {
            Some(fixture::Mode::ResizeOnInput)
        } else if argument == std::ffi::OsStr::new("--animate-and-resize-on-input") {
            Some(fixture::Mode::AnimateAndResizeOnInput)
        } else if argument == std::ffi::OsStr::new("--game-like") {
            Some(fixture::Mode::GameLike)
        } else {
            None
        };
        if let Some(parsed_mode) = parsed_mode {
            if mode.replace(parsed_mode).is_some() {
                fixture::print_usage();
                std::process::exit(2);
            }
        } else if argument == std::ffi::OsStr::new("--control-socket") {
            if control_socket.is_some() {
                fixture::print_usage();
                std::process::exit(2);
            }
            control_socket = arguments.next().map(std::path::PathBuf::from);
            if control_socket.is_none() {
                fixture::print_usage();
                std::process::exit(2);
            }
        } else if argument == std::ffi::OsStr::new("--run-nonce") {
            if run_nonce.is_some() {
                fixture::print_usage();
                std::process::exit(2);
            }
            run_nonce = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value != 0);
            if run_nonce.is_none() {
                fixture::print_usage();
                std::process::exit(2);
            }
        } else if argument == std::ffi::OsStr::new("--inactive") {
            if inactive {
                fixture::print_usage();
                std::process::exit(2);
            }
            inactive = true;
        } else if argument == std::ffi::OsStr::new("--independent-visual-stimulus") {
            if independent_visual_stimulus {
                fixture::print_usage();
                std::process::exit(2);
            }
            independent_visual_stimulus = true;
        } else if argument == std::ffi::OsStr::new("--report-execution-context") {
            report_execution_context = true;
        } else {
            fixture::print_usage();
            std::process::exit(2);
        }
    }
    if report_execution_context {
        if mode.is_some()
            || control_socket.is_some()
            || run_nonce.is_some()
            || inactive
            || independent_visual_stimulus
        {
            fixture::print_usage();
            std::process::exit(2);
        }
        fixture::report_execution_context();
        return;
    }
    if control_socket.is_some() && run_nonce.is_none() {
        fixture::print_usage();
        std::process::exit(2);
    }
    match fixture::run(
        mode.unwrap_or(fixture::Mode::Static),
        control_socket,
        run_nonce.unwrap_or_else(fixture::local_run_nonce),
        !inactive,
        independent_visual_stimulus,
    ) {
        Ok(()) => {}
        Err(status) => {
            eprintln!("macOS input fixture failed: {status}");
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "macos")]
mod fixture {
    use std::ffi::{CString, c_char, c_void};
    use std::fmt;
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::slice;
    use std::str;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::fixture_protocol::{
        BUNDLE_IDENTIFIER, EventSummary, EventTotals, FILL_RGB, FIXTURE_CONTROL_VERSION,
        FixtureCommand, FixtureCommandKind, FixtureCommandResult, FixtureMode, FixtureRenderer,
        MAX_CONTROL_LINE_BYTES, MAX_EVENT_TEXT_UNITS, MAX_RECORDED_EVENTS, REPLACEMENT_FILL_RGB,
        WINDOW_POINTS, begin_event_payload_digest, extend_event_payload_digest,
        finish_event_payload_digest, fixture_title, format_command_result_line, format_event_line,
        parse_command_line,
    };

    /// Protocol output selected once before the native run loop starts.
    static OUTPUT: OnceLock<Mutex<Box<dyn Write + Send>>> = OnceLock::new();
    /// Identity generated by the owning harness and echoed by every run record.
    static RUN_NONCE: OnceLock<u64> = OnceLock::new();
    /// Process-wide, payload-free event facts guarded across the AppKit callback.
    static EVENTS: OnceLock<Mutex<RecordedEvents>> = OnceLock::new();
    /// At most one native command may await its main-queue acknowledgement.
    static PENDING_COMMAND: OnceLock<Mutex<Option<FixtureCommand>>> = OnceLock::new();

    #[derive(Debug, Clone, Copy)]
    struct RecordedEvents {
        totals: EventTotals,
        event_payload_tag: u64,
        payload_digest: u64,
        tag_consistent: bool,
    }

    impl Default for RecordedEvents {
        fn default() -> Self {
            Self {
                totals: EventTotals::default(),
                event_payload_tag: 0,
                payload_digest: 0,
                tag_consistent: true,
            }
        }
    }

    impl RecordedEvents {
        fn correlation(self) -> u32 {
            (self.event_payload_tag >> 32) as u32
        }

        fn payload_matches(self) -> bool {
            self.totals.event_count() != 0
                && self.event_payload_tag != 0
                && self.tag_consistent
                && u64::from(finish_event_payload_digest(self.payload_digest))
                    == self.event_payload_tag & u64::from(u32::MAX)
        }

        fn record_payload(&mut self, event_payload_tag: u64, payload_fingerprint: u64) {
            if self.totals.event_count() == 0 {
                self.event_payload_tag = event_payload_tag;
                self.payload_digest = begin_event_payload_digest((event_payload_tag >> 32) as u32);
            } else {
                self.tag_consistent &= self.event_payload_tag == event_payload_tag;
            }
            self.payload_digest =
                extend_event_payload_digest(self.payload_digest, payload_fingerprint);
        }

        fn record_event(
            &mut self,
            kind: u32,
            text_units: u32,
            event_payload_tag: u64,
            payload_fingerprint: u64,
        ) -> bool {
            if !matches!(
                kind,
                crate::fixture_protocol::EVENT_POINTER_MOVE
                    | crate::fixture_protocol::EVENT_POINTER_PRESS
                    | crate::fixture_protocol::EVENT_POINTER_RELEASE
                    | crate::fixture_protocol::EVENT_POINTER_SCROLL
                    | crate::fixture_protocol::EVENT_KEY_DOWN
                    | crate::fixture_protocol::EVENT_KEY_UP
                    | crate::fixture_protocol::EVENT_FLAGS_CHANGED
            ) {
                return false;
            }
            self.record_payload(event_payload_tag, payload_fingerprint);
            let count = match kind {
                crate::fixture_protocol::EVENT_POINTER_MOVE => &mut self.totals.pointer_moves,
                crate::fixture_protocol::EVENT_POINTER_PRESS => &mut self.totals.pointer_presses,
                crate::fixture_protocol::EVENT_POINTER_RELEASE => &mut self.totals.pointer_releases,
                crate::fixture_protocol::EVENT_POINTER_SCROLL => &mut self.totals.pointer_scrolls,
                crate::fixture_protocol::EVENT_KEY_DOWN => &mut self.totals.key_downs,
                crate::fixture_protocol::EVENT_KEY_UP => &mut self.totals.key_ups,
                crate::fixture_protocol::EVENT_FLAGS_CHANGED => &mut self.totals.flags_changed,
                _ => unreachable!("the event kind was validated above"),
            };
            *count = count.saturating_add(1);
            self.totals.text_units = self.totals.text_units.saturating_add(text_units.into());
            true
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Mode {
        Static,
        Replace,
        AnimateOnInput,
        ResizeOnInput,
        AnimateAndResizeOnInput,
        GameLike,
    }

    pub(super) fn print_usage() {
        eprintln!(
            "usage: mado-pilot-macos-input-fixture \
             [--report-execution-context|--replace-window-after-ready|--game-like|\
             --animate-on-input|--resize-on-input|--animate-and-resize-on-input] \
             [--independent-visual-stimulus] [--inactive] \
             [--control-socket <path> --run-nonce <nonzero-u64>]"
        );
    }

    pub(super) fn local_run_nonce() -> u64 {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                duration.as_secs().rotate_left(32) ^ u64::from(duration.subsec_nanos())
            });
        (time ^ (u64::from(std::process::id()) << 32)).max(1)
    }

    #[derive(Debug)]
    struct ExecutionContextReport {
        launch: u32,
        signature: u32,
        signing_identifier: Option<String>,
    }

    impl ExecutionContextReport {
        fn from_raw(launch: u32, mut signature: u32, identifier: &[u8]) -> Self {
            let signing_identifier =
                if matches!(signature, SIGNATURE_AD_HOC | SIGNATURE_CERTIFICATE) {
                    match str::from_utf8(identifier) {
                        Ok(identifier) if !identifier.is_empty() => Some(identifier.to_owned()),
                        _ => {
                            signature = SIGNATURE_PLATFORM_FAILURE;
                            None
                        }
                    }
                } else {
                    None
                };
            Self {
                launch,
                signature,
                signing_identifier,
            }
        }

        fn signing_identifier(&self) -> &str {
            self.signing_identifier.as_deref().unwrap_or("none")
        }
    }

    fn execution_context() -> ExecutionContextReport {
        let mut launch = LAUNCH_UNKNOWN;
        let mut signature = SIGNATURE_PLATFORM_FAILURE;
        let mut identifier = [0u8; SIGNING_IDENTIFIER_CAPACITY];
        let mut identifier_len = 0usize;
        // SAFETY: every output is writable and the byte buffer has the capacity
        // passed beside it. This call inspects code metadata and presents no UI.
        let status = unsafe {
            mp_shim_execution_context(
                &raw mut launch,
                &raw mut signature,
                identifier.as_mut_ptr(),
                identifier.len(),
                &raw mut identifier_len,
            )
        };
        if status != OK {
            return ExecutionContextReport::from_raw(
                LAUNCH_UNKNOWN,
                SIGNATURE_PLATFORM_FAILURE,
                &[],
            );
        }
        let Some(identifier) = identifier.get(..identifier_len) else {
            return ExecutionContextReport::from_raw(launch, SIGNATURE_PLATFORM_FAILURE, &[]);
        };
        ExecutionContextReport::from_raw(launch, signature, identifier)
    }

    pub(super) fn report_execution_context() {
        let report = execution_context();
        println!(
            "fixture-context launch={} signature={} signing-identifier={}",
            launch_context_name(report.launch),
            signature_mode_name(report.signature),
            report.signing_identifier(),
        );
    }

    pub(super) fn run(
        mode: Mode,
        control_socket: Option<PathBuf>,
        run_nonce: u64,
        activate: bool,
        independent_visual_stimulus: bool,
    ) -> Result<(), Status> {
        let input: Box<dyn Read + Send> = if let Some(path) = control_socket {
            let stream = UnixStream::connect(path).map_err(|_| Status(PLATFORM_FAILURE))?;
            let input = stream.try_clone().map_err(|_| Status(PLATFORM_FAILURE))?;
            OUTPUT
                .set(Mutex::new(Box::new(stream)))
                .map_err(|_| Status(PLATFORM_FAILURE))?;
            Box::new(input)
        } else {
            OUTPUT
                .set(Mutex::new(Box::new(io::stdout())))
                .map_err(|_| Status(PLATFORM_FAILURE))?;
            Box::new(io::stdin())
        };
        RUN_NONCE
            .set(run_nonce)
            .map_err(|_| Status(PLATFORM_FAILURE))?;
        EVENTS
            .set(Mutex::new(RecordedEvents::default()))
            .map_err(|_| Status(PLATFORM_FAILURE))?;
        PENDING_COMMAND
            .set(Mutex::new(None))
            .map_err(|_| Status(PLATFORM_FAILURE))?;
        let title = fixture_title(std::process::id());
        let encoded = CString::new(title.clone()).map_err(|_| Status(INVALID_ARGUMENT))?;
        let report = execution_context();
        let signing_identifier = report
            .signing_identifier
            .as_deref()
            .map_or(&[][..], str::as_bytes);
        let replacement_delay_ms = if mode == Mode::Replace {
            REPLACEMENT_DELAY_MS
        } else {
            0
        };
        let behavior = match mode {
            Mode::AnimateOnInput => BEHAVIOR_ANIMATE_ON_INPUT,
            Mode::ResizeOnInput => BEHAVIOR_RESIZE_ON_INPUT,
            Mode::AnimateAndResizeOnInput => BEHAVIOR_ANIMATE_ON_INPUT | BEHAVIOR_RESIZE_ON_INPUT,
            Mode::Static | Mode::Replace | Mode::GameLike => 0,
        };
        let behavior = behavior
            | if independent_visual_stimulus {
                BEHAVIOR_TAGGED_INPUT_NO_VISUAL
            } else {
                0
            };
        let renderer = if mode == Mode::GameLike {
            RENDERER_OPENGL
        } else {
            RENDERER_APPKIT_BACKGROUND
        };

        let _controller = thread::Builder::new()
            .name("mado-pilot-fixture-control".to_owned())
            .spawn(move || read_control_commands(input))
            .map_err(|_| Status(PLATFORM_FAILURE))?;

        // SAFETY: `encoded` outlives the call, the four callbacks are plain
        // `extern "C"` functions that contain their own panics, the signing
        // identifier bytes outlive the call, and the context pointer is null
        // because no callback dereferences it.
        let status = unsafe {
            mp_fixture_run(
                encoded.as_ptr(),
                run_nonce,
                FILL_RGB,
                REPLACEMENT_FILL_RGB,
                behavior,
                renderer,
                replacement_delay_ms,
                WINDOW_POINTS.0,
                WINDOW_POINTS.1,
                u32::from(activate),
                report.launch,
                report.signature,
                signing_identifier.as_ptr(),
                signing_identifier.len(),
                std::ptr::null_mut(),
                on_ready,
                on_replaced,
                on_controlled,
                on_event,
            )
        };
        if status == OK {
            Ok(())
        } else {
            Err(Status(status))
        }
    }
    fn write_protocol_line(arguments: fmt::Arguments<'_>) {
        let Some(output) = OUTPUT.get() else {
            return;
        };
        let mut output = output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _written = output.write_fmt(arguments);
        let _newline = output.write_all(b"\n");
        let _flushed = output.flush();
    }

    /// Contains a Rust callback panic, including a panic from destruction of
    /// the first panic payload, before returning through an `extern "C"` seam.
    fn contain_ffi_panic(body: impl FnOnce()) {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
            Ok(()) => {}
            Err(payload) => {
                if let Err(payload) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
                {
                    let _payload = std::mem::ManuallyDrop::new(payload);
                }
            }
        }
    }

    /// Prints the line a check waits for before it discovers anything.
    ///
    /// Contains its own panics: this runs on the fixture's main thread through an
    /// `extern "C"` boundary, where an escaping panic aborts the process.
    unsafe extern "C" fn on_ready(
        _context: *mut c_void,
        window_number: u64,
        run_nonce: u64,
        renderer: u32,
        launch: u32,
        signature: u32,
        signing_identifier: *const u8,
        signing_identifier_len: usize,
    ) {
        contain_ffi_panic(|| {
            let identifier = if signing_identifier_len == 0
                || signing_identifier.is_null()
                || signing_identifier_len >= SIGNING_IDENTIFIER_CAPACITY
            {
                &[][..]
            } else {
                // SAFETY: the fixture native boundary promises this borrowed
                // view for the duration of the callback and the bound above is
                // the one the producer used.
                unsafe { slice::from_raw_parts(signing_identifier, signing_identifier_len) }
            };
            let report = ExecutionContextReport::from_raw(launch, signature, identifier);
            let (mode, renderer) = match renderer {
                RENDERER_APPKIT_BACKGROUND => {
                    (FixtureMode::Default, FixtureRenderer::AppKitBackground)
                }
                RENDERER_OPENGL => (FixtureMode::GameLike, FixtureRenderer::OpenGl),
                _ => return,
            };
            write_protocol_line(format_args!(
                "fixture-ready title={} pid={} window={window_number} run={run_nonce} \
                 control-version={} mode={} renderer={} launch={} signature={} \
                 signing-identifier={} bundle={} capacity={MAX_RECORDED_EVENTS}",
                fixture_title(std::process::id()),
                std::process::id(),
                FIXTURE_CONTROL_VERSION,
                mode.as_str(),
                renderer.as_str(),
                launch_context_name(report.launch),
                signature_mode_name(report.signature),
                report.signing_identifier(),
                BUNDLE_IDENTIFIER,
            ));
        });
    }

    /// Reports the result of the opt-in same-process window replacement.
    ///
    /// Contains its own panics for the same FFI reason as [`on_ready`].
    unsafe extern "C" fn on_replaced(
        _context: *mut c_void,
        status: u32,
        old_window_number: u64,
        new_window_number: u64,
    ) {
        contain_ffi_panic(|| {
            let run_nonce = RUN_NONCE.get().copied().unwrap_or(0);
            write_protocol_line(format_args!(
                "fixture-replaced run={run_nonce} status={status} \
                 old-window={old_window_number} new-window={new_window_number}"
            ));
        });
    }

    /// Reports one private command result without any command or input payload.
    unsafe extern "C" fn on_controlled(
        _context: *mut c_void,
        nonce: u64,
        command: u32,
        status: u32,
        before_window: u64,
        after_window: u64,
    ) {
        contain_ffi_panic(|| {
            complete_control(nonce, command, status, before_window, after_window);
        });
    }

    fn complete_control(
        nonce: u64,
        command: u32,
        status: u32,
        before_window: u64,
        after_window: u64,
    ) {
        let Some(pending) = PENDING_COMMAND.get() else {
            return;
        };
        let mut pending = pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(accepted) = pending.take() else {
            return;
        };
        if accepted.nonce != nonce || accepted.kind.as_raw() != command {
            return;
        }
        drop(pending);
        let Some(events) = EVENTS.get() else {
            return;
        };
        let mut events = events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if accepted.kind == FixtureCommandKind::ResetEvents && status == OK {
            *events = RecordedEvents::default();
        }
        let snapshot = *events;
        drop(events);
        print_control_result(FixtureCommandResult {
            run_nonce: accepted.run_nonce,
            nonce,
            status,
            before_window,
            after_window,
            event_correlation: snapshot.correlation(),
            event_payload_matches: snapshot.payload_matches(),
            events: snapshot.totals,
        });
    }

    fn print_control_result(result: FixtureCommandResult) {
        write_protocol_line(format_args!("{}", format_command_result_line(result)));
    }

    /// Reads bounded, payload-free commands from the owned harness.
    ///
    /// One byte at a time is intentionally boring here: the fixture is not a
    /// throughput path, and this keeps malformed lines from allocating beyond
    /// the protocol's fixed bound.
    fn read_control_commands(mut input: Box<dyn Read + Send>) {
        let mut line = Vec::with_capacity(MAX_CONTROL_LINE_BYTES);
        let mut byte = [0u8; 1];
        let mut overflow = false;
        loop {
            match input.read(&mut byte) {
                Ok(0) | Err(_) => {
                    if let Some(run_nonce) = RUN_NONCE.get().copied() {
                        terminate_after_control_close(run_nonce);
                    }
                    return;
                }
                Ok(_) if byte[0] == b'\n' => {
                    if !overflow
                        && let Ok(text) = str::from_utf8(&line)
                        && let Some(command) = parse_command_line(text)
                        && RUN_NONCE.get() == Some(&command.run_nonce)
                    {
                        let Some(pending) = PENDING_COMMAND.get() else {
                            return;
                        };
                        let mut pending = pending
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if pending.is_some() {
                            drop(pending);
                            write_protocol_line(format_args!(
                                "fixture-command-rejected run={} nonce={} status={INVALID_ARGUMENT}",
                                command.run_nonce, command.nonce
                            ));
                        } else {
                            *pending = Some(command);
                            drop(pending);
                            // SAFETY: the scalars are the complete private native API.
                            let status = unsafe {
                                mp_fixture_control(
                                    FIXTURE_CONTROL_VERSION,
                                    command.run_nonce,
                                    command.nonce,
                                    command.kind.as_raw(),
                                    command.event_payload_tag,
                                )
                            };
                            if status != OK {
                                complete_control(
                                    command.nonce,
                                    command.kind.as_raw(),
                                    status,
                                    0,
                                    0,
                                );
                            }
                            if command.kind == FixtureCommandKind::Stop && status == OK {
                                return;
                            }
                        }
                    } else {
                        write_protocol_line(format_args!(
                            "fixture-command-rejected status={INVALID_ARGUMENT}"
                        ));
                    }
                    line.clear();
                    overflow = false;
                }
                Ok(_) if !overflow && line.len() < MAX_CONTROL_LINE_BYTES => line.push(byte[0]),
                Ok(_) => overflow = true,
            }
        }
    }

    fn terminate_after_control_close(run_nonce: u64) {
        // SAFETY: the scalars are the complete private native API and the
        // native boundary validates the run identity before terminating.
        let status = unsafe { mp_fixture_control_closed(FIXTURE_CONTROL_VERSION, run_nonce) };
        if status != OK {
            // The harness is gone, so a native run loop that cannot accept its
            // termination request must not remain as an unaffiliated process.
            std::process::exit(1);
        }
    }

    /// Prints one bounded event summary. Never prints what was typed.
    unsafe extern "C" fn on_event(
        _context: *mut c_void,
        kind: u32,
        text_units: u32,
        event_payload_tag: u64,
        payload_fingerprint: u64,
    ) {
        contain_ffi_panic(|| {
            let Some(run_nonce) = RUN_NONCE.get().copied() else {
                return;
            };
            let Some(events) = EVENTS.get() else {
                return;
            };
            let mut events = events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if events.totals.event_count() >= MAX_RECORDED_EVENTS as u64 {
                events.totals.saturated = true;
                return;
            }
            let bounded_units = text_units.min(MAX_EVENT_TEXT_UNITS);
            events.totals.saturated |= bounded_units != text_units;
            if !events.record_event(kind, bounded_units, event_payload_tag, payload_fingerprint) {
                events.totals.saturated = true;
                return;
            }
            let reported = events.totals.event_count();
            let correlation = events.correlation();
            drop(events);
            write_protocol_line(format_args!(
                "{}",
                format_event_line(
                    run_nonce,
                    EventSummary {
                        kind,
                        text_units: bounded_units,
                        correlation,
                    }
                )
            ));
            if reported == MAX_RECORDED_EVENTS as u64 {
                write_protocol_line(format_args!("fixture-capacity-reached run={run_nonce}"));
            }
        });
    }

    const fn launch_context_name(launch: u32) -> &'static str {
        match launch {
            LAUNCH_BUNDLED => "bundled",
            LAUNCH_UNBUNDLED => "unbundled",
            _ => "unknown",
        }
    }

    const fn signature_mode_name(signature: u32) -> &'static str {
        match signature {
            1 => "unsigned",
            2 => "invalid",
            SIGNATURE_AD_HOC => "ad-hoc",
            SIGNATURE_CERTIFICATE => "certificate-backed",
            _ => "platform-failure",
        }
    }

    const OK: u32 = 0;
    const INVALID_ARGUMENT: u32 = 1;
    const UNSUPPORTED: u32 = 2;
    const LAUNCH_UNKNOWN: u32 = 0;
    const LAUNCH_BUNDLED: u32 = 1;
    const LAUNCH_UNBUNDLED: u32 = 2;
    const SIGNATURE_PLATFORM_FAILURE: u32 = 0;
    const SIGNATURE_AD_HOC: u32 = 3;
    const SIGNATURE_CERTIFICATE: u32 = 4;
    const SIGNING_IDENTIFIER_CAPACITY: usize = 256;
    const REPLACEMENT_DELAY_MS: u32 = 5_000;
    const BEHAVIOR_ANIMATE_ON_INPUT: u32 = 1;
    const BEHAVIOR_RESIZE_ON_INPUT: u32 = 2;
    const BEHAVIOR_TAGGED_INPUT_NO_VISUAL: u32 = 4;
    const RENDERER_APPKIT_BACKGROUND: u32 = 0;
    const RENDERER_OPENGL: u32 = 1;
    const PLATFORM_FAILURE: u32 = 3;

    /// A native status the fixture could not start under.
    pub(super) struct Status(u32);

    impl fmt::Display for Status {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let detail = match self.0 {
                INVALID_ARGUMENT => "the fixture was asked for a window it cannot create",
                UNSUPPORTED => "this host does not provide the requested desktop renderer",
                3 => "the platform refused to create the fixture window",
                4 => "a native exception was contained while starting the fixture",
                _ => "the fixture reported an unrecognized status",
            };
            write!(formatter, "{detail} ({})", self.0)
        }
    }

    unsafe extern "C" {
        fn mp_fixture_run(
            title: *const c_char,
            run_nonce: u64,
            fill: u32,
            replacement_fill: u32,
            behavior: u32,
            renderer: u32,
            replacement_delay_ms: u32,
            width: f64,
            height: f64,
            activate: u32,
            launch_context: u32,
            signature_mode: u32,
            signing_identifier: *const u8,
            signing_identifier_len: usize,
            context: *mut c_void,
            ready: unsafe extern "C" fn(*mut c_void, u64, u64, u32, u32, u32, *const u8, usize),
            replaced: unsafe extern "C" fn(*mut c_void, u32, u64, u64),
            controlled: unsafe extern "C" fn(*mut c_void, u64, u32, u32, u64, u64),
            sink: unsafe extern "C" fn(*mut c_void, u32, u32, u64, u64),
        ) -> u32;
        fn mp_fixture_control(
            version: u32,
            run_nonce: u64,
            nonce: u64,
            command: u32,
            event_payload_tag: u64,
        ) -> u32;
        fn mp_fixture_control_closed(version: u32, run_nonce: u64) -> u32;
        #[cfg(test)]
        fn mp_fixture_test_unsupported_renderer() -> u32;
        fn mp_shim_execution_context(
            out_launch: *mut u32,
            out_signature: *mut u32,
            out_identifier: *mut u8,
            identifier_capacity: usize,
            out_identifier_len: *mut usize,
        ) -> u32;
    }

    #[cfg(test)]
    mod tests {
        use super::{RecordedEvents, UNSUPPORTED, mp_fixture_test_unsupported_renderer};
        use crate::fixture_protocol::{EVENT_KEY_DOWN, EVENT_KEY_UP, event_payload_activity_tag};

        #[test]
        fn unsupported_renderer_loading_fails_closed() {
            // SAFETY: the fixture-private probe accepts no arguments, attempts
            // only its compiled-in missing absolute path, and returns a scalar.
            let status = unsafe { mp_fixture_test_unsupported_renderer() };
            assert_eq!(status, UNSUPPORTED);
        }

        #[test]
        fn first_recorded_event_preserves_row_correlation_and_payload_digest() {
            let fingerprints = [11, 22];
            let activity_tag = event_payload_activity_tag(7, &fingerprints);
            let mut events = RecordedEvents::default();

            assert!(events.record_event(EVENT_KEY_DOWN, 0, activity_tag, fingerprints[0]));
            assert!(events.record_event(EVENT_KEY_UP, 0, activity_tag, fingerprints[1]));

            assert_eq!(events.correlation(), 7);
            assert!(events.payload_matches());
            assert_eq!(events.totals.key_downs, 1);
            assert_eq!(events.totals.key_ups, 1);
        }
    }
}
