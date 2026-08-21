//! Dedicated, deterministic target for interactive Windows input verification.
//!
//! The fixture accepts only the platform adapter's bounded, versioned
//! `WM_COPYDATA` protocol. It retains event summaries, never input text, and
//! publishes an exact PID-qualified title for fail-closed target selection.

#[cfg(not(windows))]
fn main() {
    eprintln!("mado-pilot-windows-input-fixture requires Windows");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    let options = match fixture::Options::from_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("Windows input fixture failed: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = fixture::run(options) {
        eprintln!("Windows input fixture failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod fixture {
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::io::{self, Write};
    use std::mem::size_of;
    use std::slice;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use mado_pilot_platform_windows::fixture_protocol::{
        ACKNOWLEDGED, BENCHMARK_FILL_RGB, CLASS_NAME, COPYDATA_TAG, EVENT_KEY_DOWN,
        EVENT_POINTER_MOVE, EventSummary, FILL_RGB, MAX_PACKET_BYTES, MAX_RECORDED_EVENTS,
        fixture_title, format_event_line, is_query, summarize,
    };
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, KillTimer,
        MSG, PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOZORDER, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WM_COPYDATA,
        WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP,
    };
    use windows::core::{Error, PCWSTR, Result};

    static RECORDS: Mutex<VecDeque<EventSummary>> = Mutex::new(VecDeque::new());
    static ANIMATE_ON_INPUT: AtomicBool = AtomicBool::new(false);
    static RESIZE_ON_INPUT: AtomicBool = AtomicBool::new(false);
    static CONTINUOUS_ANIMATION: AtomicBool = AtomicBool::new(false);
    static LARGE_WINDOW: AtomicBool = AtomicBool::new(false);
    static CURRENT_FILL_RGB: AtomicU32 = AtomicU32::new(FILL_RGB);
    static REPORTED_EVENTS: AtomicU32 = AtomicU32::new(0);
    const RESIZE_REPAINT_TIMER: usize = 1;
    const CONTINUOUS_ANIMATION_TIMER: usize = 2;
    const CONTINUOUS_ANIMATION_INTERVAL_MS: u32 = 16;
    const RESIZE_REPAINT_INTERVAL_MS: u32 = 16;
    const RESIZE_REPAINT_COUNT: u32 = 4;
    static RESIZE_REPAINTS_REMAINING: AtomicU32 = AtomicU32::new(0);

    #[derive(Debug, Clone, Copy, Default)]
    pub(super) struct Options {
        animate_on_input: bool,
        resize_on_input: bool,
        production_capture: bool,
    }

    impl Options {
        pub(super) fn from_args() -> std::result::Result<Self, String> {
            let mut options = Self::default();
            for argument in std::env::args().skip(1) {
                match argument.as_str() {
                    "--animate-on-input" => options.animate_on_input = true,
                    "--resize-on-input" => options.resize_on_input = true,
                    "--animate-and-resize-on-input" => {
                        options.animate_on_input = true;
                        options.resize_on_input = true;
                    }
                    "--production-capture" => options.production_capture = true,
                    _ => {
                        return Err(format!(
                            "unknown argument `{argument}`; expected --animate-on-input, \
                             --resize-on-input, --animate-and-resize-on-input, or \
                             --production-capture"
                        ));
                    }
                }
            }
            Ok(options)
        }
    }

    #[repr(C)]
    struct CopyData {
        tag: usize,
        bytes: u32,
        data: *const c_void,
    }

    pub(super) fn run(options: Options) -> Result<()> {
        if options.production_capture {
            // SAFETY: production mode fixes process DPI awareness before this
            // thread calls any DPI-sensitive USER32 function.
            unsafe {
                SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
            }
        }
        ANIMATE_ON_INPUT.store(options.animate_on_input, Ordering::Release);
        RESIZE_ON_INPUT.store(options.resize_on_input, Ordering::Release);
        CONTINUOUS_ANIMATION.store(options.production_capture, Ordering::Release);
        LARGE_WINDOW.store(false, Ordering::Release);
        CURRENT_FILL_RGB.store(FILL_RGB, Ordering::Release);
        REPORTED_EVENTS.store(0, Ordering::Release);
        RESIZE_REPAINTS_REMAINING.store(0, Ordering::Release);
        let class_name = wide(CLASS_NAME);
        let title_text = fixture_title(std::process::id());
        let title = wide(&title_text);
        // SAFETY: null requests the current process module.
        let module = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).expect("WNDCLASSEXW fits u32"),
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: the class fields are initialized and registration copies the
        // class name before this function can return.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(Error::from_thread());
        }
        let (extended_style, style, width, height) = if options.production_capture {
            (WS_EX_TOPMOST, WS_POPUP, 1_280, 720)
        } else {
            (Default::default(), WS_OVERLAPPEDWINDOW, 640, 420)
        };
        // SAFETY: the registered class and title remain alive for this call;
        // the fixture supplies no parent, menu, or application-owned payload.
        let hwnd = unsafe {
            CreateWindowExW(
                extended_style,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                160,
                160,
                width,
                height,
                None,
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
        }?;
        // SAFETY: hwnd is a live top-level window owned by this thread.
        unsafe {
            let _was_visible = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        if options.production_capture {
            // SAFETY: hwnd is live on its owning GUI thread. A null callback
            // posts deterministic animation ticks to this same window.
            if unsafe {
                SetTimer(
                    Some(hwnd),
                    CONTINUOUS_ANIMATION_TIMER,
                    CONTINUOUS_ANIMATION_INTERVAL_MS,
                    None,
                )
            } == 0
            {
                return Err(Error::from_thread());
            }
        }

        let mut output = io::stdout().lock();
        let _written = writeln!(
            output,
            "fixture-ready class={CLASS_NAME} title={title_text} capacity={MAX_RECORDED_EVENTS}"
        );
        let _flushed = output.flush();
        drop(output);

        let mut message = MSG::default();
        loop {
            // SAFETY: message is writable and this thread owns the window queue.
            let status = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if status.0 == -1 {
                return Err(Error::from_thread());
            }
            if status.0 == 0 {
                break;
            }
            // SAFETY: GetMessageW initialized message for this queue.
            unsafe {
                let _translated = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        sender: WPARAM,
        payload: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COPYDATA => receive_packet(hwnd, payload),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_TIMER if sender.0 == RESIZE_REPAINT_TIMER => {
                repaint_after_resize(hwnd);
                LRESULT(0)
            }
            WM_TIMER if sender.0 == CONTINUOUS_ANIMATION_TIMER => {
                animate_and_repaint(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: invoked on the owning GUI thread during destruction.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            _ => {
                // SAFETY: unhandled messages retain the default Win32 behavior.
                unsafe { DefWindowProcW(hwnd, message, sender, payload) }
            }
        }
    }

    fn receive_packet(hwnd: HWND, payload: LPARAM) -> LRESULT {
        let address = payload.0.cast_unsigned();
        if address == 0 {
            return LRESULT(0);
        }
        let copy_ptr = std::ptr::with_exposed_provenance::<CopyData>(address);
        // SAFETY: Windows supplies a COPYDATASTRUCT-compatible object for the
        // duration of synchronous WM_COPYDATA dispatch.
        let copy = unsafe { &*copy_ptr };
        let Ok(bytes) = usize::try_from(copy.bytes) else {
            return LRESULT(0);
        };
        if copy.tag != COPYDATA_TAG || bytes == 0 || bytes > MAX_PACKET_BYTES || copy.data.is_null()
        {
            return LRESULT(0);
        }
        // SAFETY: WM_COPYDATA guarantees `data` is readable for `bytes` during
        // this synchronous dispatch; the independent cap bounds all parsing.
        let packet = unsafe { slice::from_raw_parts(copy.data.cast::<u8>(), bytes) };
        let Some(summary) = summarize(packet) else {
            return LRESULT(0);
        };
        if !is_query(summary) {
            let Ok(mut records) = RECORDS.lock() else {
                return LRESULT(0);
            };
            if records.len() == MAX_RECORDED_EVENTS {
                records.pop_front();
            }
            records.push_back(summary);
            let animates = ANIMATE_ON_INPUT.load(Ordering::Acquire);
            if animates && summary.kind == EVENT_KEY_DOWN {
                apply_benchmark_animation();
            }
            let resize_event = RESIZE_ON_INPUT.load(Ordering::Acquire)
                && if animates {
                    summary.kind == EVENT_POINTER_MOVE
                } else {
                    summary.kind == EVENT_KEY_DOWN
                };
            if resize_event {
                apply_benchmark_resize(hwnd);
            }
            if (ANIMATE_ON_INPUT.load(Ordering::Acquire) || RESIZE_ON_INPUT.load(Ordering::Acquire))
                && REPORTED_EVENTS.fetch_add(1, Ordering::AcqRel)
                    < u32::try_from(MAX_RECORDED_EVENTS).unwrap_or(u32::MAX)
            {
                let mut output = io::stdout().lock();
                let _written = writeln!(output, "{}", format_event_line(summary));
                let _flushed = output.flush();
            }
            drop(records);
            // SAFETY: hwnd is live during message dispatch; invalidation only
            // schedules repaint of the fixture's controlled client surface.
            let _invalidated =
                unsafe { windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false) };
        }
        LRESULT(ACKNOWLEDGED.cast_signed())
    }

    fn paint(hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: called only for WM_PAINT with a writable PAINTSTRUCT.
        let device = unsafe { BeginPaint(hwnd, &raw mut paint) };
        let mut client = Default::default();
        // SAFETY: hwnd is live and client is writable for the current rectangle.
        let has_client = unsafe { GetClientRect(hwnd, &raw mut client) }.is_ok();
        if has_client {
            // COLORREF is 0x00bbggrr. The selected deterministic fill contains
            // no desktop or input payload.
            // SAFETY: creating and deleting this process-owned brush is paired.
            let brush =
                unsafe { CreateSolidBrush(colorref(CURRENT_FILL_RGB.load(Ordering::Acquire))) };
            // SAFETY: device, rectangle, and brush are live for this paint call.
            let _filled = unsafe { FillRect(device, &raw const client, brush) };
            // SAFETY: the brush is no longer selected or needed after FillRect.
            let _deleted = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
        }
        // SAFETY: balances BeginPaint for this WM_PAINT dispatch.
        let _ended = unsafe { EndPaint(hwnd, &raw const paint) };
    }

    fn apply_benchmark_animation() {
        let previous = CURRENT_FILL_RGB.load(Ordering::Acquire);
        let next = if previous == FILL_RGB {
            BENCHMARK_FILL_RGB
        } else {
            FILL_RGB
        };
        CURRENT_FILL_RGB.store(next, Ordering::Release);
    }

    fn animate_and_repaint(hwnd: HWND) {
        if !CONTINUOUS_ANIMATION.load(Ordering::Acquire) {
            return;
        }
        apply_benchmark_animation();
        // SAFETY: hwnd is live during timer dispatch; invalidation schedules
        // one controlled repaint without exposing unrelated desktop content.
        let _invalidated =
            unsafe { windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false) };
    }

    fn apply_benchmark_resize(hwnd: HWND) {
        let was_large = LARGE_WINDOW.fetch_xor(true, Ordering::AcqRel);
        let (width, height) = if was_large { (640, 420) } else { (820, 540) };
        // SAFETY: hwnd is the live fixture window. This changes only its
        // size and deliberately preserves focus, position, and z-order.
        let resized = unsafe {
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width,
                height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if resized.is_err() {
            return;
        }

        RESIZE_REPAINTS_REMAINING.store(RESIZE_REPAINT_COUNT, Ordering::Release);
        // SAFETY: hwnd is live on its owning GUI thread. A null callback posts
        // bounded WM_TIMER messages back to this same window procedure.
        if unsafe {
            SetTimer(
                Some(hwnd),
                RESIZE_REPAINT_TIMER,
                RESIZE_REPAINT_INTERVAL_MS,
                None,
            )
        } == 0
        {
            RESIZE_REPAINTS_REMAINING.store(0, Ordering::Release);
        }
    }

    fn repaint_after_resize(hwnd: HWND) {
        let remaining = RESIZE_REPAINTS_REMAINING
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_sub(1)
            })
            .unwrap_or(0);
        if remaining == 0 {
            return;
        }

        apply_benchmark_animation();
        // SAFETY: hwnd is live during message dispatch; invalidation schedules
        // one controlled repaint without exposing unrelated desktop content.
        let _invalidated =
            unsafe { windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false) };
        if remaining == 1 {
            // SAFETY: this is the owning GUI thread and the timer identifier is
            // the one created by apply_benchmark_resize for this window.
            let _killed = unsafe { KillTimer(Some(hwnd), RESIZE_REPAINT_TIMER) };
        }
    }

    fn colorref(rgb: u32) -> COLORREF {
        let red = rgb & 0xff_0000;
        let green = rgb & 0x00_ff00;
        let blue = rgb & 0x00_00ff;
        COLORREF((red >> 16) | green | (blue << 16))
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
}
