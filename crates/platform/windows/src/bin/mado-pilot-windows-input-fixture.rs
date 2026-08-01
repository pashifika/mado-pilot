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
    if let Err(error) = fixture::run() {
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

    use mado_pilot_platform_windows::fixture_protocol::{
        ACKNOWLEDGED, CLASS_NAME, COPYDATA_TAG, EventSummary, MAX_PACKET_BYTES,
        MAX_RECORDED_EVENTS, fixture_title, is_query, summarize,
    };
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, PAINTSTRUCT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, MSG,
        PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage,
        WM_COPYDATA, WM_DESTROY, WM_PAINT, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::{Error, PCWSTR, Result};

    static RECORDS: Mutex<VecDeque<EventSummary>> = Mutex::new(VecDeque::new());

    #[repr(C)]
    struct CopyData {
        tag: usize,
        bytes: u32,
        data: *const c_void,
    }

    pub(super) fn run() -> Result<()> {
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
        // SAFETY: the registered class and title remain alive for this call;
        // the fixture supplies no parent, menu, or application-owned payload.
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                160,
                160,
                640,
                420,
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
            // COLORREF is 0x00bbggrr. The fixed fill is deterministic and
            // contains no desktop or input payload.
            // SAFETY: creating and deleting this process-owned brush is paired.
            let brush = unsafe { CreateSolidBrush(COLORREF(0x0060_4020)) };
            // SAFETY: device, rectangle, and brush are live for this paint call.
            let _filled = unsafe { FillRect(device, &raw const client, brush) };
            // SAFETY: the brush is no longer selected or needed after FillRect.
            let _deleted = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
        }
        // SAFETY: balances BeginPaint for this WM_PAINT dispatch.
        let _ended = unsafe { EndPaint(hwnd, &raw const paint) };
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
}
