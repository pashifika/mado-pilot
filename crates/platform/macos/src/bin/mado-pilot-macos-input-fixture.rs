//! Dedicated, deterministic target for interactive macOS input verification.
//!
//! The fixture shows one window filled with a single fixed colour, publishes an
//! exact process-qualified title for fail-closed selection, and prints a bounded
//! summary of the events that reach it. It retains no characters: a key event is
//! reported as its kind and its UTF-16 unit count.
//!
//! macOS has no target-directed input channel, so there is nothing here that
//! accepts a packet. Everything this fixture observes arrived as ordinary
//! system input, which is the only route the macOS Adapter submits through.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("mado-pilot-macos-input-fixture requires macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let argument = arguments.next();
    if arguments.next().is_some() {
        fixture::print_usage();
        std::process::exit(2);
    }
    let mode = match argument.as_deref() {
        None => fixture::Mode::Static,
        Some(argument) if argument == std::ffi::OsStr::new("--report-execution-context") => {
            fixture::report_execution_context();
            return;
        }
        Some(argument) if argument == std::ffi::OsStr::new("--replace-window-after-ready") => {
            fixture::Mode::Replace
        }
        Some(argument) if argument == std::ffi::OsStr::new("--animate-on-input") => {
            fixture::Mode::AnimateOnInput
        }
        Some(argument) if argument == std::ffi::OsStr::new("--resize-on-input") => {
            fixture::Mode::ResizeOnInput
        }
        Some(argument) if argument == std::ffi::OsStr::new("--animate-and-resize-on-input") => {
            fixture::Mode::AnimateAndResizeOnInput
        }
        Some(_) => {
            fixture::print_usage();
            std::process::exit(2);
        }
    };
    match fixture::run(mode) {
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
    use std::io::{self, Write};
    use std::slice;
    use std::str;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mado_pilot_platform_macos::fixture_protocol::{
        BUNDLE_IDENTIFIER, EventSummary, FILL_RGB, MAX_RECORDED_EVENTS, REPLACEMENT_FILL_RGB,
        WINDOW_POINTS, fixture_title, format_event_line,
    };

    /// How many events have been reported, so reporting stays bounded.
    static REPORTED: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Mode {
        Static,
        Replace,
        AnimateOnInput,
        ResizeOnInput,
        AnimateAndResizeOnInput,
    }

    pub(super) fn print_usage() {
        eprintln!(
            "usage: mado-pilot-macos-input-fixture \
             [--report-execution-context|--replace-window-after-ready|\
             --animate-on-input|--resize-on-input|--animate-and-resize-on-input]"
        );
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

    pub(super) fn run(mode: Mode) -> Result<(), Status> {
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
            Mode::Static | Mode::Replace => 0,
        };

        // SAFETY: `encoded` outlives the call, the three callbacks are plain
        // `extern "C"` functions that contain their own panics, the signing
        // identifier bytes outlive the call, and the context pointer is null
        // because no callback dereferences it.
        let status = unsafe {
            mp_fixture_run(
                encoded.as_ptr(),
                FILL_RGB,
                REPLACEMENT_FILL_RGB,
                behavior,
                replacement_delay_ms,
                WINDOW_POINTS.0,
                WINDOW_POINTS.1,
                report.launch,
                report.signature,
                signing_identifier.as_ptr(),
                signing_identifier.len(),
                std::ptr::null_mut(),
                on_ready,
                on_replaced,
                on_event,
            )
        };
        if status == OK {
            Ok(())
        } else {
            Err(Status(status))
        }
    }

    /// Prints the line a check waits for before it discovers anything.
    ///
    /// Contains its own panics: this runs on the fixture's main thread through an
    /// `extern "C"` boundary, where an escaping panic aborts the process.
    unsafe extern "C" fn on_ready(
        _context: *mut c_void,
        window_number: u64,
        launch: u32,
        signature: u32,
        signing_identifier: *const u8,
        signing_identifier_len: usize,
    ) {
        let _contained = std::panic::catch_unwind(|| {
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
            let mut output = io::stdout().lock();
            let _written = writeln!(
                output,
                "fixture-ready title={} pid={} window={window_number} launch={} signature={} \
                 signing-identifier={} bundle={} capacity={MAX_RECORDED_EVENTS}",
                fixture_title(std::process::id()),
                std::process::id(),
                launch_context_name(report.launch),
                signature_mode_name(report.signature),
                report.signing_identifier(),
                BUNDLE_IDENTIFIER,
            );
            let _flushed = output.flush();
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
        let _contained = std::panic::catch_unwind(|| {
            let mut output = io::stdout().lock();
            let _written = writeln!(
                output,
                "fixture-replaced status={status} old-window={old_window_number} \
                 new-window={new_window_number}"
            );
            let _flushed = output.flush();
        });
    }

    /// Prints one bounded event summary. Never prints what was typed.
    unsafe extern "C" fn on_event(_context: *mut c_void, kind: u32, text_units: u32) {
        let _contained = std::panic::catch_unwind(|| {
            let reported = REPORTED.fetch_add(1, Ordering::AcqRel);
            if reported >= MAX_RECORDED_EVENTS {
                // The counter keeps rising and nothing further is printed, so a
                // long-running fixture cannot fill a check's pipe.
                return;
            }
            let mut output = io::stdout().lock();
            let _written = writeln!(
                output,
                "{}",
                format_event_line(EventSummary { kind, text_units })
            );
            if reported + 1 == MAX_RECORDED_EVENTS {
                let _capacity = writeln!(output, "fixture-capacity-reached");
            }
            let _flushed = output.flush();
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

    /// A native status the fixture could not start under.
    pub(super) struct Status(u32);

    impl fmt::Display for Status {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let detail = match self.0 {
                INVALID_ARGUMENT => "the fixture was asked for a window it cannot create",
                2 => "this host does not provide the desktop framework the fixture needs",
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
            fill: u32,
            replacement_fill: u32,
            behavior: u32,
            replacement_delay_ms: u32,
            width: f64,
            height: f64,
            launch_context: u32,
            signature_mode: u32,
            signing_identifier: *const u8,
            signing_identifier_len: usize,
            context: *mut c_void,
            ready: unsafe extern "C" fn(*mut c_void, u64, u32, u32, *const u8, usize),
            replaced: unsafe extern "C" fn(*mut c_void, u32, u64, u64),
            sink: unsafe extern "C" fn(*mut c_void, u32, u32),
        ) -> u32;
        fn mp_shim_execution_context(
            out_launch: *mut u32,
            out_signature: *mut u32,
            out_identifier: *mut u8,
            identifier_capacity: usize,
            out_identifier_len: *mut usize,
        ) -> u32;
    }
}
