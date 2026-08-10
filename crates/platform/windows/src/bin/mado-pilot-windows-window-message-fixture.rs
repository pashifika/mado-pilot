//! Ordinary-window native fixture for `WindowMessage` evidence.
//!
//! The fixture reports only bounded counters and message families. It never
//! prints message payloads, native handles, process identifiers, or input text.

#[cfg(not(windows))]
fn main() {
    eprintln!("mado-pilot-windows-window-message-fixture requires Windows");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    let options = match fixture::options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = fixture::run(options) {
        eprintln!("ordinary input fixture failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod fixture {
    use std::ffi::c_void;
    use std::io::{self, Write};
    use std::mem::size_of;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    use mado_pilot_platform_windows::fixture_protocol::{
        BENCHMARK_FILL_RGB, CONTROL_ALLOW_FOREGROUND, CONTROL_BLOCK_QUEUE, CONTROL_DESTROY_TARGET,
        CONTROL_DUPLICATE_METADATA, CONTROL_REPARENT_TARGET, CONTROL_REPLACE_TARGET,
        CONTROL_REPORT, CONTROL_REUSE_STRESS, CONTROL_SET_GEOMETRY, FILL_RGB, MAX_RECORDED_EVENTS,
        ORDINARY_CLASS_NAME, ordinary_fixture_title,
    };
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HGDIOBJ, InvalidateRect,
        PAINTSTRUCT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F6};
    use windows::Win32::UI::Input::{RAWINPUTDEVICE, RegisterRawInputDevices};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        GetClientRect, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, MSG,
        PostQuitMessage, RegisterClassExW, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
        SWP_NOZORDER, SetForegroundWindow, SetParent, SetTimer, SetWindowPos, SetWindowTextW,
        ShowWindow, WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_INPUT, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
        WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{Error, PCWSTR, Result};

    const STATE_TIMER: usize = 1;
    const STATE_POLL_INTERVAL_MS: u32 = 5;

    static TITLE_TOKEN: OnceLock<String> = OnceLock::new();
    static TARGET: AtomicUsize = AtomicUsize::new(0);
    static GAME: AtomicUsize = AtomicUsize::new(0);
    static SIBLING: AtomicUsize = AtomicUsize::new(0);
    static CHILD: AtomicUsize = AtomicUsize::new(0);
    static FOREGROUND: AtomicUsize = AtomicUsize::new(0);
    static RAW: AtomicUsize = AtomicUsize::new(0);
    static STATE: AtomicUsize = AtomicUsize::new(0);
    static REPLACEMENT_ACTIVE: AtomicBool = AtomicBool::new(false);
    static LAST_F6_DOWN: AtomicBool = AtomicBool::new(false);
    static ANIMATED: AtomicBool = AtomicBool::new(false);
    static GEOMETRY_REPAINTS: AtomicU32 = AtomicU32::new(0);

    static TARGET_EVENTS: AtomicU32 = AtomicU32::new(0);
    static REPLACEMENT_EVENTS: AtomicU32 = AtomicU32::new(0);
    static GAME_EVENTS: AtomicU32 = AtomicU32::new(0);
    static SIBLING_EVENTS: AtomicU32 = AtomicU32::new(0);
    static CHILD_EVENTS: AtomicU32 = AtomicU32::new(0);
    static FOREGROUND_EVENTS: AtomicU32 = AtomicU32::new(0);
    static RAW_LEGACY_EVENTS: AtomicU32 = AtomicU32::new(0);
    static STATE_LEGACY_EVENTS: AtomicU32 = AtomicU32::new(0);
    static RAW_EVENTS: AtomicU32 = AtomicU32::new(0);
    static STATE_CHANGES: AtomicU32 = AtomicU32::new(0);

    pub(super) struct Options {
        token: String,
        activate: bool,
    }

    pub(super) fn options() -> std::result::Result<Options, String> {
        let mut token = None;
        let mut activate = false;
        for argument in std::env::args().skip(1) {
            if argument == "--activate" {
                if activate {
                    return Err("--activate may be supplied only once".to_owned());
                }
                activate = true;
                continue;
            }
            let Some(value) = argument.strip_prefix("--title-token=") else {
                return Err(format!(
                    "unknown argument `{argument}`; expected --title-token=<token> or --activate"
                ));
            };
            if value.is_empty() || value.chars().count() > 64 {
                return Err("title token must contain 1..=64 characters".to_owned());
            }
            if token.replace(value.to_owned()).is_some() {
                return Err("--title-token may be supplied only once".to_owned());
            }
        }
        Ok(Options {
            token: token.unwrap_or_else(|| std::process::id().to_string()),
            activate,
        })
    }

    fn activate_for_fixture_setup(window: HWND) -> Result<()> {
        // The native matrix must own its unrelated foreground application even
        // when an unattended host's foreground lock rejects a direct request.
        // Queue attachment is confined to this explicit fixture-startup mode
        // and is detached before readiness or any delivery observation.
        // SAFETY: both identifiers name desktop GUI threads with message queues,
        // and every successful attachment is detached before this function exits.
        unsafe {
            let current_thread = GetCurrentThreadId();
            let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let attached = foreground_thread != 0 && foreground_thread != current_thread;
            if attached {
                AttachThreadInput(current_thread, foreground_thread, true).ok()?;
            }
            let _was_visible = ShowWindow(window, SW_SHOW);
            let activated = SetForegroundWindow(window).ok();
            let detached = if attached {
                AttachThreadInput(current_thread, foreground_thread, false).ok()
            } else {
                Ok(())
            };
            activated?;
            detached
        }
    }

    pub(super) fn run(options: Options) -> Result<()> {
        // SAFETY: DPI awareness is selected before this fixture calls USER32.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        }
        TITLE_TOKEN.set(options.token).map_err(|_| {
            Error::new(
                windows::core::HRESULT(0x8000_4005u32.cast_signed()),
                "title initialized twice",
            )
        })?;
        ANIMATED.store(false, Ordering::Release);
        let class_name = wide(ORDINARY_CLASS_NAME);
        // SAFETY: null requests the current executable module.
        let module = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).expect("WNDCLASSEXW fits u32"),
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: registration copies the class name, and every field is initialized.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(Error::from_thread());
        }

        let target = create_top_level(&ordinary_title(), 120, 120)?;
        let game = create_top_level(&role_title("Game"), 1_200, 120)?;
        let sibling = create_top_level(&role_title("Sibling"), 800, 120)?;
        let child = create_child(sibling, &role_title("Child"))?;
        let foreground = create_top_level(&role_title("Foreground"), 800, 520)?;
        let raw = create_top_level(&role_title("Raw"), 120, 560)?;
        let state = create_top_level(&role_title("State"), 460, 560)?;

        TARGET.store(handle_value(target), Ordering::Release);
        GAME.store(handle_value(game), Ordering::Release);
        SIBLING.store(handle_value(sibling), Ordering::Release);
        CHILD.store(handle_value(child), Ordering::Release);
        FOREGROUND.store(handle_value(foreground), Ordering::Release);
        RAW.store(handle_value(raw), Ordering::Release);
        STATE.store(handle_value(state), Ordering::Release);

        for window in [target, game, sibling, foreground, raw, state] {
            // SAFETY: each handle is a live top-level window owned by this thread.
            unsafe {
                let _was_visible = ShowWindow(window, SW_SHOWNOACTIVATE);
            }
        }

        if options.activate {
            activate_for_fixture_setup(target)?;
        }

        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 2,
                dwFlags: Default::default(),
                hwndTarget: raw,
            },
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: Default::default(),
                hwndTarget: raw,
            },
        ];
        // SAFETY: the array and structure size match this process ABI.
        unsafe {
            RegisterRawInputDevices(
                &devices,
                u32::try_from(size_of::<RAWINPUTDEVICE>()).expect("RAWINPUTDEVICE fits u32"),
            )?;
        }
        // SAFETY: the state window is live on this thread; a null callback posts WM_TIMER.
        if unsafe { SetTimer(Some(state), STATE_TIMER, STATE_POLL_INTERVAL_MS, None) } == 0 {
            return Err(Error::from_thread());
        }

        print_line(&format!(
            "fixture-ready class={ORDINARY_CLASS_NAME} title={} capacity={MAX_RECORDED_EVENTS}",
            ordinary_title()
        ));

        let mut message = MSG::default();
        loop {
            // SAFETY: message is writable and this thread owns every fixture window.
            let status = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if status.0 == -1 {
                return Err(Error::from_thread());
            }
            if status.0 == 0 {
                break;
            }
            // Deliberately do not call TranslateMessage: the production route
            // posts every key and text unit explicitly and does not synthesize WM_CHAR.
            // SAFETY: GetMessageW initialized the scalar message structure.
            unsafe {
                DispatchMessageW(&raw const message);
            }
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            CONTROL_REPORT => {
                print_report();
                LRESULT(0)
            }
            CONTROL_DUPLICATE_METADATA => {
                let sibling = load_handle(&SIBLING);
                let title = wide(&ordinary_title());
                // SAFETY: sibling is retained while the fixture runs; Windows copies title.
                let changed = unsafe { SetWindowTextW(sibling, PCWSTR(title.as_ptr())) }.is_ok();
                print_line(if changed {
                    "control duplicate-metadata=ready"
                } else {
                    "control duplicate-metadata=failed"
                });
                LRESULT(0)
            }
            CONTROL_REPARENT_TARGET => {
                let sibling = load_handle(&SIBLING);
                // SAFETY: both windows belong to this GUI thread. A null prior parent is a
                // valid successful return that the generated Result wrapper may reject.
                let _previous = unsafe { SetParent(hwnd, Some(sibling)) };
                print_line("control reparent=ready");
                LRESULT(0)
            }
            CONTROL_ALLOW_FOREGROUND => {
                let process_id =
                    u32::try_from(wparam.0).expect("foreground process identifier fits u32");
                // SAFETY: the owned fixture is foreground and delegates only to its test host.
                let delegated = unsafe { AllowSetForegroundWindow(process_id) }.is_ok();
                print_line(if delegated {
                    "control foreground-delegate=ready"
                } else {
                    "control foreground-delegate=failed"
                });
                LRESULT(0)
            }
            CONTROL_REPLACE_TARGET => {
                replace_target(hwnd);
                LRESULT(0)
            }
            CONTROL_REUSE_STRESS => {
                reuse_stress(hwnd, wparam.0);
                LRESULT(0)
            }
            CONTROL_SET_GEOMETRY => {
                let position = u64::try_from(wparam.0).expect("WPARAM fits u64");
                let size = u64::try_from(lparam.0.cast_unsigned()).expect("LPARAM fits u64");
                let x = u32::try_from(position & u64::from(u32::MAX))
                    .expect("masked position fits u32")
                    .cast_signed();
                let y = u32::try_from(position >> 32)
                    .expect("shifted position fits u32")
                    .cast_signed();
                let width = u32::try_from(size & u64::from(u32::MAX))
                    .expect("masked size fits u32")
                    .cast_signed();
                let height = u32::try_from(size >> 32)
                    .expect("shifted size fits u32")
                    .cast_signed();
                // SAFETY: the retained target is live and this is its owning GUI thread.
                let updated = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                }
                .is_ok();
                if updated {
                    GEOMETRY_REPAINTS.store(64, Ordering::Release);
                }
                let repainted = updated && pulse_target_paint();
                print_line(if repainted {
                    "control geometry=ready"
                } else {
                    "control geometry=failed"
                });
                LRESULT(0)
            }
            CONTROL_DESTROY_TARGET => {
                // SAFETY: control is dispatched only to the live retained target.
                let destroyed = unsafe { DestroyWindow(hwnd) }.is_ok();
                print_line(if destroyed {
                    "control target-loss=ready"
                } else {
                    "control target-loss=failed"
                });
                LRESULT(0)
            }
            CONTROL_BLOCK_QUEUE => {
                // SAFETY: the timer belongs to the state window on this same GUI
                // thread. Removing it keeps queue-capacity rows deterministic.
                let _killed = unsafe { KillTimer(Some(load_handle(&STATE)), STATE_TIMER) };
                let milliseconds = u64::try_from(wparam.0).unwrap_or(u64::MAX).min(60_000);
                print_line("control queue-block=ready");
                std::thread::sleep(Duration::from_millis(milliseconds));
                print_line("control queue-block=complete");
                LRESULT(0)
            }
            WM_INPUT if handle_value(hwnd) == RAW.load(Ordering::Acquire) => {
                bounded_increment(&RAW_EVENTS);
                print_observation("raw", "raw-input");
                LRESULT(0)
            }
            WM_TIMER
                if handle_value(hwnd) == STATE.load(Ordering::Acquire)
                    && wparam.0 == STATE_TIMER =>
            {
                if GEOMETRY_REPAINTS
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    let _repainted = pulse_target_paint();
                }
                // SAFETY: polling one documented virtual key reads process-global state only.
                let down = unsafe { GetAsyncKeyState(i32::from(VK_F6.0)) } < 0;
                let previous = LAST_F6_DOWN.swap(down, Ordering::AcqRel);
                if previous != down {
                    bounded_increment(&STATE_CHANGES);
                    print_observation("state", "async-state-change");
                }
                LRESULT(0)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => observe_legacy(hwnd, "pointer-move"),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                observe_legacy(hwnd, "button-down")
            }
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
                observe_legacy(hwnd, "button-up")
            }
            WM_MOUSEWHEEL => observe_legacy(hwnd, "vertical-wheel"),
            WM_MOUSEHWHEEL => observe_legacy(hwnd, "horizontal-wheel"),
            WM_KEYDOWN => observe_legacy(hwnd, "key-down"),
            WM_KEYUP => observe_legacy(hwnd, "key-up"),
            WM_CHAR => observe_legacy(hwnd, "text-unit"),
            WM_CLOSE => {
                // SAFETY: hwnd is live during dispatch on its owning thread.
                let _destroyed = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_DESTROY => LRESULT(0),
            WM_NCDESTROY => {
                clear_handle(hwnd);
                LRESULT(0)
            }
            _ => {
                // SAFETY: unhandled scalar messages retain default Win32 behavior.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }

    fn observe_legacy(hwnd: HWND, family: &'static str) -> LRESULT {
        let (role, counter) = role(hwnd);
        bounded_increment(counter);
        print_observation(role, family);
        if role == "target" && !ANIMATED.swap(true, Ordering::AcqRel) {
            // SAFETY: hwnd is live during dispatch; invalidation schedules only
            // the fixture's controlled client surface for repaint.
            let _invalidated = unsafe { InvalidateRect(Some(hwnd), None, false) };
        }
        LRESULT(0)
    }

    fn role(hwnd: HWND) -> (&'static str, &'static AtomicU32) {
        let value = handle_value(hwnd);
        if value == TARGET.load(Ordering::Acquire) {
            if REPLACEMENT_ACTIVE.load(Ordering::Acquire) {
                ("replacement", &REPLACEMENT_EVENTS)
            } else {
                ("target", &TARGET_EVENTS)
            }
        } else if value == GAME.load(Ordering::Acquire) {
            ("game", &GAME_EVENTS)
        } else if value == SIBLING.load(Ordering::Acquire) {
            ("sibling", &SIBLING_EVENTS)
        } else if value == CHILD.load(Ordering::Acquire) {
            ("child", &CHILD_EVENTS)
        } else if value == FOREGROUND.load(Ordering::Acquire) {
            ("foreground", &FOREGROUND_EVENTS)
        } else if value == RAW.load(Ordering::Acquire) {
            ("raw-legacy", &RAW_LEGACY_EVENTS)
        } else if value == STATE.load(Ordering::Acquire) {
            ("state-legacy", &STATE_LEGACY_EVENTS)
        } else {
            ("other", &FOREGROUND_EVENTS)
        }
    }

    fn replace_target(hwnd: HWND) {
        let old = handle_value(hwnd);
        // SAFETY: control is dispatched to the live target on its owning thread.
        if unsafe { DestroyWindow(hwnd) }.is_err() {
            print_line("control replacement=failed");
            return;
        }
        match create_top_level(&ordinary_title(), 120, 120) {
            Ok(replacement) => {
                let reused = handle_value(replacement) == old;
                install_replacement(replacement);
                print_line(if reused {
                    "control replacement=ready reused=true"
                } else {
                    "control replacement=ready reused=false"
                });
            }
            Err(_) => print_line("control replacement=failed"),
        }
    }

    fn reuse_stress(hwnd: HWND, requested: usize) {
        let retained = handle_value(hwnd);
        let bound = requested.clamp(1, 4_096);
        // SAFETY: control is dispatched to the live target on its owning thread.
        if unsafe { DestroyWindow(hwnd) }.is_err() {
            print_line("control reuse-stress=failed");
            return;
        }
        for iteration in 1..=bound {
            let Ok(candidate) = create_top_level(&ordinary_title(), 120, 120) else {
                print_line("control reuse-stress=failed");
                return;
            };
            if handle_value(candidate) == retained {
                install_replacement(candidate);
                print_line(&format!(
                    "control reuse-stress=ready iterations={iteration} observed=true"
                ));
                return;
            }
            // SAFETY: candidate was just created on this GUI thread and is not published.
            if unsafe { DestroyWindow(candidate) }.is_err() {
                print_line("control reuse-stress=failed");
                return;
            }
        }
        match create_top_level(&ordinary_title(), 120, 120) {
            Ok(replacement) => {
                install_replacement(replacement);
                print_line(&format!(
                    "control reuse-stress=ready iterations={bound} observed=false"
                ));
            }
            Err(_) => print_line("control reuse-stress=failed"),
        }
    }

    fn install_replacement(replacement: HWND) {
        TARGET.store(handle_value(replacement), Ordering::Release);
        REPLACEMENT_ACTIVE.store(true, Ordering::Release);
        // SAFETY: the replacement is live and owned by this thread.
        unsafe {
            let _was_visible = ShowWindow(replacement, SW_SHOWNOACTIVATE);
        }
    }

    fn pulse_target_paint() -> bool {
        let target = load_handle(&TARGET);
        ANIMATED.fetch_xor(true, Ordering::AcqRel);
        // SAFETY: the retained target is owned by this GUI thread.
        unsafe { InvalidateRect(Some(target), None, true) }.as_bool()
            && unsafe { UpdateWindow(target) }.as_bool()
    }

    fn print_report() {
        print_line(&format!(
            "report target={} replacement={} game={} sibling={} child={} foreground={} raw-legacy={} state-legacy={} raw={} state={}",
            TARGET_EVENTS.load(Ordering::Acquire),
            REPLACEMENT_EVENTS.load(Ordering::Acquire),
            GAME_EVENTS.load(Ordering::Acquire),
            SIBLING_EVENTS.load(Ordering::Acquire),
            CHILD_EVENTS.load(Ordering::Acquire),
            FOREGROUND_EVENTS.load(Ordering::Acquire),
            RAW_LEGACY_EVENTS.load(Ordering::Acquire),
            STATE_LEGACY_EVENTS.load(Ordering::Acquire),
            RAW_EVENTS.load(Ordering::Acquire),
            STATE_CHANGES.load(Ordering::Acquire),
        ));
    }

    fn paint(hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: called only for WM_PAINT with a writable PAINTSTRUCT.
        let device = unsafe { BeginPaint(hwnd, &raw mut paint) };
        let mut client = Default::default();
        // SAFETY: hwnd is live and client is writable for the current rectangle.
        if unsafe { GetClientRect(hwnd, &raw mut client) }.is_ok() {
            let fill = if handle_value(hwnd) == TARGET.load(Ordering::Acquire)
                && ANIMATED.load(Ordering::Acquire)
            {
                BENCHMARK_FILL_RGB
            } else {
                FILL_RGB
            };
            // SAFETY: creating, using, and deleting this process-owned brush is paired.
            let brush = unsafe { CreateSolidBrush(colorref(fill)) };
            // SAFETY: device, rectangle, and brush are live for this paint call.
            let _filled = unsafe { FillRect(device, &raw const client, brush) };
            // SAFETY: the brush is no longer selected or needed after FillRect.
            let _deleted = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
        }
        // SAFETY: balances BeginPaint for this WM_PAINT dispatch.
        let _ended = unsafe { EndPaint(hwnd, &raw const paint) };
    }

    fn colorref(rgb: u32) -> COLORREF {
        let red = rgb & 0xff_0000;
        let green = rgb & 0x00_ff00;
        let blue = rgb & 0x00_00ff;
        COLORREF((red >> 16) | green | (blue << 16))
    }

    fn print_observation(role: &str, family: &str) {
        print_line(&format!("observation role={role} family={family} units=1"));
    }

    fn print_line(line: &str) {
        let mut output = io::stdout().lock();
        let _written = writeln!(output, "{line}");
        let _flushed = output.flush();
    }

    fn bounded_increment(counter: &AtomicU32) {
        let ceiling = u32::try_from(MAX_RECORDED_EVENTS).unwrap_or(u32::MAX);
        let _updated = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            (value < ceiling).then_some(value + 1)
        });
    }

    fn create_top_level(title: &str, x: i32, y: i32) -> Result<HWND> {
        create_window(title, x, y, 360, 240, None, WS_OVERLAPPEDWINDOW)
    }

    fn create_child(parent: HWND, title: &str) -> Result<HWND> {
        create_window(title, 20, 20, 160, 100, Some(parent), WS_CHILD | WS_VISIBLE)
    }

    fn create_window(
        title: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Option<HWND>,
        style: WINDOW_STYLE,
    ) -> Result<HWND> {
        let class_name = wide(ORDINARY_CLASS_NAME);
        let title = wide(title);
        // SAFETY: null requests the current executable module.
        let module = unsafe { GetModuleHandleW(None) }?;
        // SAFETY: class and title buffers remain alive for this call; no payload is supplied.
        unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                x,
                y,
                width,
                height,
                parent,
                None,
                Some(HINSTANCE(module.0)),
                None,
            )
        }
    }

    fn ordinary_title() -> String {
        ordinary_fixture_title(TITLE_TOKEN.get().expect("title token initialized"))
    }

    fn role_title(role: &str) -> String {
        format!(
            "MadoPilot Ordinary WindowMessage {role} [{}]",
            TITLE_TOKEN.get().expect("title token initialized")
        )
    }

    fn handle_value(hwnd: HWND) -> usize {
        hwnd.0.addr()
    }

    fn load_handle(handle: &AtomicUsize) -> HWND {
        HWND(std::ptr::with_exposed_provenance_mut::<c_void>(
            handle.load(Ordering::Acquire),
        ))
    }

    fn clear_handle(hwnd: HWND) {
        let value = handle_value(hwnd);
        for handle in [&TARGET, &GAME, &SIBLING, &CHILD, &FOREGROUND, &RAW, &STATE] {
            let _cleared = handle.compare_exchange(value, 0, Ordering::AcqRel, Ordering::Acquire);
        }
        if [&TARGET, &GAME, &SIBLING, &FOREGROUND, &RAW, &STATE]
            .iter()
            .all(|handle| handle.load(Ordering::Acquire) == 0)
        {
            // SAFETY: all top-level fixture windows are gone on this GUI thread.
            unsafe { PostQuitMessage(0) };
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
}
