//! Dedicated, deterministic target for interactive macOS input verification.
//!
//! The fixture shows one window filled with a single fixed colour, publishes an
//! exact process-qualified title for fail-closed selection, and prints a bounded
//! summary of the events that reach it. It retains no characters: a key event is
//! reported as its kind and its UTF-16 unit count.
//!
//! macOS has no background input channel, so there is nothing here that accepts a
//! packet. Everything this fixture observes arrived as ordinary system input,
//! which is the only kind the macOS Adapter delivers.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("mado-pilot-macos-input-fixture requires macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
fn main() {
    match fixture::run() {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mado_pilot_platform_macos::fixture_protocol::{
        BUNDLE_IDENTIFIER, EventSummary, FILL_RGB, MAX_RECORDED_EVENTS, WINDOW_POINTS,
        fixture_title, format_event_line,
    };

    /// How many events have been reported, so reporting stays bounded.
    static REPORTED: AtomicUsize = AtomicUsize::new(0);

    pub(super) fn run() -> Result<(), Status> {
        let title = fixture_title(std::process::id());
        let encoded = CString::new(title.clone()).map_err(|_| Status(INVALID_ARGUMENT))?;

        // SAFETY: `encoded` outlives the call, the two callbacks are plain
        // `extern "C"` functions that contain their own panics, and the context
        // pointer is null because neither of them dereferences it.
        let status = unsafe {
            mp_fixture_run(
                encoded.as_ptr(),
                FILL_RGB,
                WINDOW_POINTS.0,
                WINDOW_POINTS.1,
                std::ptr::null_mut(),
                on_ready,
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
    unsafe extern "C" fn on_ready(_context: *mut c_void, window_number: u64, launch: u32) {
        let _contained = std::panic::catch_unwind(|| {
            let mut output = io::stdout().lock();
            let _written = writeln!(
                output,
                "fixture-ready title={} pid={} window={window_number} context={} bundle={} \
                 capacity={MAX_RECORDED_EVENTS}",
                fixture_title(std::process::id()),
                std::process::id(),
                launch_context_name(launch),
                BUNDLE_IDENTIFIER,
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
            1 => "bundled",
            2 => "unbundled",
            _ => "unknown",
        }
    }

    const OK: u32 = 0;
    const INVALID_ARGUMENT: u32 = 1;

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
            width: f64,
            height: f64,
            context: *mut c_void,
            ready: unsafe extern "C" fn(*mut c_void, u64, u32),
            sink: unsafe extern "C" fn(*mut c_void, u32, u32),
        ) -> u32;
    }
}
