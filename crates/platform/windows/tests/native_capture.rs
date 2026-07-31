#![cfg(windows)]
//! Native WGC coverage against a test-owned synthetic Win32 window.

use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Once};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mado_pilot_capture::{
    CaptureProvider, DiscoveryRequest, FrameRequest, OpenRequest, PixelFormat,
};
use mado_pilot_core::{FrameSequence, IdentityIssuer, OperationContext, Status, TargetKind};
use mado_pilot_platform_windows::{PROVIDER, WindowsCaptureProvider};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, InvalidateRect,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, MSG,
    PM_REMOVE, PeekMessageW, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, TranslateMessage, WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::PCWSTR;

static PAINT_COLOR: AtomicU32 = AtomicU32::new(0x0020_4080);
static TEST_STAGE: AtomicU32 = AtomicU32::new(0);
static REGISTER_SYNTHETIC_CLASS: Once = Once::new();

fn stage(value: u32) {
    TEST_STAGE.store(value, Ordering::Release);
}

fn stage_name(value: u32) -> &'static str {
    match value {
        0 => "starting",
        1 => "fixture-ready",
        2 => "discovered",
        3 => "ordinary-close",
        4 => "opened",
        5 => "retained-progress",
        6 => "mapped",
        7 => "resized",
        8 => "moved",
        9 => "target-closed",
        10 => "loss-observed",
        11 => "first-close",
        12 => "second-close",
        13 => "replacement-lifecycle",
        _ => "complete",
    }
}

struct SyntheticWindow {
    resize: Arc<AtomicBool>,
    move_window: Arc<AtomicBool>,
    close: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SyntheticWindow {
    fn spawn() -> Self {
        let resize = Arc::new(AtomicBool::new(false));
        let move_window = Arc::new(AtomicBool::new(false));
        let close = Arc::new(AtomicBool::new(false));
        let thread_resize = Arc::clone(&resize);
        let thread_move = Arc::clone(&move_window);
        let thread_close = Arc::clone(&close);
        let thread = thread::spawn(move || {
            let class_name = wide("MadoPilotPhase2NativeCaptureTestClass");
            let title = wide("MadoPilot Phase 2 native capture target");
            // SAFETY: null requests the current process module.
            let module = unsafe { GetModuleHandleW(None) }.expect("module");
            REGISTER_SYNTHETIC_CLASS.call_once(|| {
                let class = WNDCLASSEXW {
                    cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).expect("class size"),
                    lpfnWndProc: Some(window_proc),
                    hInstance: HINSTANCE(module.0),
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    ..WNDCLASSEXW::default()
                };
                // SAFETY: class fields and string storage remain valid through
                // registration, which copies the class definition process-wide.
                let atom = unsafe { RegisterClassExW(&raw const class) };
                assert_ne!(atom, 0, "registered synthetic class");
            });
            // SAFETY: the registered class and title pointers are valid; no
            // parent, menu, or application-owned lparam is supplied.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    PCWSTR(class_name.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    120,
                    120,
                    640,
                    420,
                    None,
                    None,
                    Some(HINSTANCE(module.0)),
                    None,
                )
            }
            .expect("created synthetic target");
            // SAFETY: hwnd is a live window owned by this thread.
            unsafe {
                let _shown = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }

            while !thread_close.load(Ordering::Acquire) {
                PAINT_COLOR.fetch_add(0x0001_0307, Ordering::Relaxed);
                // SAFETY: invalidating the owned window schedules a paint and
                // does not activate it.
                let _invalidated = unsafe { InvalidateRect(Some(hwnd), None, true) };
                if thread_resize.swap(false, Ordering::AcqRel) {
                    // SAFETY: resize preserves z-order and focus.
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            120,
                            120,
                            820,
                            540,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        )
                    }
                    .expect("resized synthetic target");
                }
                if thread_move.swap(false, Ordering::AcqRel) {
                    // SAFETY: movement preserves size, z-order, and focus.
                    unsafe {
                        SetWindowPos(
                            hwnd,
                            None,
                            260,
                            180,
                            820,
                            540,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        )
                    }
                    .expect("moved synthetic target");
                }
                pump_messages();
                thread::sleep(Duration::from_millis(16));
            }
            // SAFETY: hwnd remains owned by this thread until destruction.
            unsafe { DestroyWindow(hwnd) }.expect("destroyed synthetic target");
            pump_messages();
        });
        // Give the window thread time to register, show, and paint once before
        // picker-free discovery snapshots current targets.
        thread::sleep(Duration::from_millis(100));
        Self {
            resize,
            move_window,
            close,
            thread: Some(thread),
        }
    }

    fn resize(&self) {
        self.resize.store(true, Ordering::Release);
    }

    fn move_window(&self) {
        self.move_window.store(true, Ordering::Release);
    }

    fn close_target(&mut self) {
        self.close.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("synthetic window thread");
        }
    }
}

impl Drop for SyntheticWindow {
    fn drop(&mut self) {
        if thread::panicking() {
            self.close.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _joined = thread.join();
            }
        } else {
            self.close_target();
        }
    }
}

#[test]
fn synthetic_window_exercises_retention_resize_loss_and_close() {
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(10));
        let stage = TEST_STAGE.load(Ordering::Acquire);
        if stage != u32::MAX {
            eprintln!(
                "native-capture watchdog expired at stage {stage} ({})",
                stage_name(stage)
            );
            std::process::exit(101);
        }
    });
    let mut target_window = SyntheticWindow::spawn();
    stage(1);
    let issuer = Arc::new(IdentityIssuer::new());
    let provider = WindowsCaptureProvider::new(Arc::clone(&issuer));
    let request = DiscoveryRequest::new()
        .with_kind(TargetKind::Window)
        .with_name_containing("MadoPilot Phase 2 native capture target");
    let targets = match provider.discover_matching(&request, &timed()) {
        Ok(targets) => targets,
        Err(error) if error.status() == Status::Unsupported => {
            eprintln!("SKIP native WGC contract: runtime capability unavailable");
            return;
        }
        Err(error) => panic!("native discovery failed: {error}"),
    };
    let target = targets
        .first()
        .expect("supported WGC discovers its test-owned synthetic target");
    stage(2);
    assert_eq!(target.provider(), PROVIDER);
    assert_eq!(target.capability().kind(), Some(TargetKind::Window));
    assert!(
        target
            .coordinates()
            .supports(mado_pilot_core::CoordinateSpace::DesktopLogical)
    );

    let unsupported = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Rgba8),
            &timed(),
        )
        .expect_err("WGC publishes one fixed BGRA format");
    assert_eq!(unsupported.status(), Status::Unsupported);

    let ordinary_close = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect("opened ordinary-close session");
    let ordinary_frame = ordinary_close
        .frame(&FrameRequest::latest(), &timed())
        .expect("ordinary-close frame");
    let ordinary_close = thread::spawn(move || {
        ordinary_close
            .close(&close_timed())
            .expect("cross-thread ordinary close initializes WinRT");
        ordinary_close
    })
    .join()
    .expect("cross-thread close worker");
    ordinary_close
        .close(&close_timed())
        .expect("ordinary close is idempotent");
    ordinary_frame
        .map(PixelFormat::Bgra8, &timed())
        .expect("ordinary-close frame remains mappable");

    let implicit_drop = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect("opened implicit-drop session");
    let implicit_frame = implicit_drop
        .frame(&FrameRequest::latest(), &timed())
        .expect("implicit-drop frame");
    thread::spawn(move || drop(implicit_drop))
        .join()
        .expect("cross-thread implicit drop");
    implicit_frame
        .map(PixelFormat::Bgra8, &timed())
        .expect("implicit-drop frame remains mappable");
    stage(3);

    let pressure = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect("opened finite-pressure session");
    let mut retained = vec![
        pressure
            .frame(&FrameRequest::latest(), &timed())
            .expect("first retained pressure frame"),
    ];
    while retained.len() < 40 {
        let next = pressure
            .frame(
                &FrameRequest::newer_than(retained.last().expect("retained frame").stamp()),
                &timed(),
            )
            .expect("filled finite retained-storage budget");
        retained.push(next);
    }
    let before_pressure = retained.last().expect("last retained frame").stamp();
    let blocked = pressure
        .frame(&FrameRequest::newer_than(before_pressure), &short_timed())
        .expect_err("full retained budget publishes no invented frame");
    assert_eq!(blocked.status(), Status::DeadlineExceeded);
    retained.remove(0);
    let resumed = pressure
        .frame(&FrameRequest::newer_than(before_pressure), &timed())
        .expect("release resumes finite producer progress");
    assert!(
        resumed.stamp().sequence().value() > before_pressure.sequence().value() + 1,
        "pressure drops become an observable sequence gap"
    );
    pressure
        .close(&close_timed())
        .expect("pressure session close");
    drop(retained);
    drop(resumed);

    let session = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect("opened synthetic WGC session");
    stage(4);
    assert_eq!(session.description().format(), PixelFormat::Bgra8);
    assert_eq!(
        session
            .description()
            .queue()
            .retained_storage()
            .map(|value| value.get()),
        Some(40)
    );

    let first = session
        .frame(&FrameRequest::latest(), &timed())
        .expect("first frame");
    let mut progressed = first.clone();
    for _ in 0..3 {
        progressed = session
            .frame(&FrameRequest::newer_than(progressed.stamp()), &timed())
            .expect("producer progressed beyond its two-frame pool");
    }
    let second = progressed;
    stage(5);
    let retained_mapping = first
        .map(PixelFormat::Bgra8, &timed())
        .expect("retained frame mapped lazily");
    assert_eq!(
        retained_mapping.bytes().len(),
        retained_mapping.descriptor().byte_len()
    );
    assert!(!retained_mapping.is_shared());
    stage(6);

    let old_extent = second.descriptor().extent();
    target_window.resize();
    let mut previous = second;
    let resized = loop {
        let candidate = session
            .frame(&FrameRequest::newer_than(previous.stamp()), &timed())
            .expect("frame after resize request");
        if candidate.descriptor().extent() != old_extent {
            break candidate;
        }
        previous = candidate;
    };
    assert_eq!(resized.stamp().sequence(), FrameSequence::FIRST);
    assert!(resized.stamp().epoch() > first.stamp().epoch());
    assert!(resized.stamp().geometry() > first.stamp().geometry());
    assert_eq!(
        resized.transform().geometry(),
        resized.stamp().geometry(),
        "frame identity and its authoritative placement revision stay correlated"
    );
    assert!(
        resized
            .transform()
            .supports(mado_pilot_core::CoordinateSpace::DesktopLogical)
    );
    stage(7);

    let before_move_origin = resized
        .transform()
        .target()
        .expect("window placement")
        .desktop_origin();
    target_window.move_window();
    let mut previous = resized;
    let moved = loop {
        let candidate = session
            .frame(&FrameRequest::newer_than(previous.stamp()), &timed())
            .expect("frame after movement request");
        let origin = candidate
            .transform()
            .target()
            .expect("moved window placement")
            .desktop_origin();
        if origin != before_move_origin {
            break candidate;
        }
        previous = candidate;
    };
    assert_eq!(
        moved.stamp().epoch(),
        previous.stamp().epoch(),
        "movement changes geometry without resetting the resized epoch"
    );
    assert!(moved.stamp().geometry() > previous.stamp().geometry());
    stage(8);

    let registry_before_loss = provider_registry_counts(&provider);
    target_window.close_target();
    stage(9);
    let mut stamp = moved.stamp();
    loop {
        match session.frame(&FrameRequest::newer_than(stamp), &timed()) {
            Ok(late) => stamp = late.stamp(),
            Err(error) => {
                assert_eq!(error.status(), Status::TargetLost);
                break;
            }
        }
    }
    stage(10);

    let stale = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect_err("the provider rejects and reclaims its lost live record");
    assert_eq!(stale.status(), Status::TargetLost);
    let reclaimed = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect_err("an accepted identity absent after reclamation stays lost");
    assert_eq!(reclaimed.status(), Status::TargetLost);
    let registry_after_reclamation = provider_registry_counts(&provider);
    assert_eq!(
        registry_after_reclamation,
        (
            registry_before_loss
                .0
                .checked_sub(1)
                .expect("the live target was registered"),
            registry_before_loss
                .1
                .checked_sub(1)
                .expect("the live native key was registered"),
        ),
        "stale open removes exactly its native record instead of retaining a tombstone"
    );

    session.close(&close_timed()).expect("first close");
    stage(11);
    session.close(&close_timed()).expect("idempotent close");
    stage(12);
    assert!(session.is_closed());
    assert_eq!(
        retained_mapping.bytes().len(),
        retained_mapping.descriptor().byte_len(),
        "mapping survives target loss and session close"
    );

    stage(13);
    let _replacement_window = SyntheticWindow::spawn();
    let replacements = provider
        .discover_matching(&request, &timed())
        .expect("replacement discovery");
    let replacement = replacements
        .first()
        .expect("replacement synthetic target is discoverable");
    assert_ne!(
        replacement.id(),
        target.id(),
        "a replacement target receives a new identity"
    );
    let stale_after_replacement = provider
        .open(
            target.id(),
            &OpenRequest::new().require_format(PixelFormat::Bgra8),
            &timed(),
        )
        .expect_err("the old identity never opens its replacement");
    assert_eq!(stale_after_replacement.status(), Status::TargetLost);
    TEST_STAGE.store(u32::MAX, Ordering::Release);
}

fn provider_registry_counts(provider: &WindowsCaptureProvider) -> (usize, usize) {
    let debug = format!("{provider:?}");
    (
        debug_count(&debug, "known_targets"),
        debug_count(&debug, "current_targets"),
    )
}

fn debug_count(debug: &str, field: &str) -> usize {
    let marker = format!("{field}: ");
    debug
        .split_once(&marker)
        .and_then(|(_, tail)| tail.split([',', ' ']).next())
        .expect("provider debug field")
        .parse()
        .expect("provider debug count")
}

fn timed() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_secs(5))
        .expect("deadline representable")
}

fn close_timed() -> OperationContext {
    // Native teardown remains deadline-bounded, but a saturated workspace test
    // host can take longer than an ordinary frame request to schedule the
    // apartment-initialized close worker.
    OperationContext::new()
        .with_timeout(Duration::from_secs(20))
        .expect("deadline representable")
}

fn short_timed() -> OperationContext {
    OperationContext::new()
        .with_timeout(Duration::from_millis(200))
        .expect("deadline representable")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn pump_messages() {
    let mut message = MSG::default();
    // SAFETY: message is writable, and every returned message is translated and
    // dispatched before the next poll.
    while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: message was initialized by PeekMessageW.
        unsafe {
            let _translated = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: paint is writable and paired with EndPaint below.
        let device_context = unsafe { BeginPaint(hwnd, &raw mut paint) };
        let mut client = windows::Win32::Foundation::RECT::default();
        // SAFETY: client is writable for the live window.
        let _rect = unsafe { GetClientRect(hwnd, &raw mut client) };
        // SAFETY: COLORREF is a plain Win32 BGR value.
        let brush = unsafe { CreateSolidBrush(COLORREF(PAINT_COLOR.load(Ordering::Relaxed))) };
        // SAFETY: all handles and the client rectangle are valid for this paint.
        unsafe {
            FillRect(device_context, &raw const client, brush);
            let _deleted = DeleteObject(HGDIOBJ(brush.0));
            let _ended = EndPaint(hwnd, &raw const paint);
        }
        return LRESULT(0);
    }
    // SAFETY: unhandled messages are delegated unchanged to the system default.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
