//! Ordinary-window native fixture for `WindowMessage` evidence.
//!
//! Payload-free observations are the default. Opt-in coordinate and input oracles
//! report only owned fixture positions and scalar counts, never text or handles.

#[cfg(not(windows))]
fn main() {
    eprintln!("mado-pilot-windows-window-message-fixture requires Windows");
    std::process::exit(2);
}

#[cfg(windows)]
#[path = "support/ordinary_fixture_startup.rs"]
mod ordinary_fixture_startup;

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
        fixture::report_error(&error);
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod fixture {
    use std::ffi::c_void;
    use std::io::{self, Write};
    use std::mem::size_of;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::ordinary_fixture_startup::{
        ActivationPath, Context, DpiAfterFailure, DpiMode, DpiScope, Failure, Foreground, Stage,
        Status, activate_with_fallback, ready_record,
    };

    use mado_pilot_platform_windows::fixture_protocol::{
        BENCHMARK_FILL_RGB, CONTROL_BLOCK_QUEUE, CONTROL_DESTROY_TARGET,
        CONTROL_DUPLICATE_METADATA, CONTROL_REPARENT_TARGET, CONTROL_REPLACE_TARGET,
        CONTROL_REPORT, CONTROL_REUSE_STRESS, CONTROL_SET_GEOMETRY, CONTROL_SET_VISUAL_ABSENT,
        CONTROL_SET_VISUAL_VISIBLE, CONTROL_TRANSITION_VISUAL, FILL_RGB, FixtureVisualCommand,
        MAX_RECORDED_EVENTS, ORDINARY_CLASS_NAME, TARGET_LOSS_ACKNOWLEDGEMENT,
        VISUAL_TRANSITION_ACKNOWLEDGEMENT, WATCH_MARKER_CELL_SIZE, WATCH_MARKER_HEIGHT,
        WATCH_MARKER_PRIMARY_RGB, WATCH_MARKER_SECONDARY_RGB, WATCH_MARKER_WIDTH, WATCH_MARKER_X,
        WATCH_MARKER_Y, WATCH_TOKEN_CELL_COUNT, WATCH_TOKEN_CELL_SIZE, WATCH_TOKEN_GRID_WIDTH,
        WATCH_TOKEN_X, WATCH_TOKEN_Y, ordinary_fixture_title, visual_command_for_control,
        visual_token_cell,
    };
    use windows::Win32::Foundation::{
        COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError,
        WIN32_ERROR, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HDC,
        HGDIOBJ, InvalidateRect, PAINTSTRUCT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE, DPI_AWARENESS_UNAWARE,
        GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
        LogicalToPhysicalPointForPerMonitorDPI, SetProcessDpiAwarenessContext,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_F6, VK_LSHIFT, VK_PACKET, VK_RSHIFT, VK_SHIFT,
    };
    use windows::Win32::UI::Input::{RAWINPUTDEVICE, RegisterRawInputDevices};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, MSG,
        PostQuitMessage, RegisterClassExW, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
        SWP_NOZORDER, SetForegroundWindow, SetParent, SetTimer, SetWindowPos, SetWindowTextW,
        ShowWindow, TranslateMessage, WINDOW_STYLE, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_INPUT,
        WM_KEYDOWN, WM_KEYFIRST, WM_KEYLAST, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY,
        WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
        WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };
    use windows::core::{Error, PCWSTR, Result as WindowsResult};

    const STATE_TIMER: usize = 1;
    const STATE_POLL_INTERVAL_MS: u32 = 5;
    const INPUT_ORACLE_KEYS: [(u32, u16); 4] = [
        (WM_KEYDOWN, VK_SHIFT.0),
        (WM_KEYDOWN, VK_F6.0),
        (WM_KEYUP, VK_F6.0),
        (WM_KEYUP, VK_SHIFT.0),
    ];
    const INPUT_ORACLE_TEXT: [u16; 3] = [0x0041, 0xd83d, 0xde42];

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
    static WATCH_VISUAL_STATE: AtomicU64 = AtomicU64::new(0);
    static COORDINATE_ORACLE: AtomicBool = AtomicBool::new(false);
    static INPUT_ORACLE: OnceLock<InputOracle> = OnceLock::new();

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

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum InputOracleMode {
        WindowMessage,
        System,
    }

    struct InputOracle {
        mode: InputOracleMode,
        target: usize,
        keys: AtomicU32,
        packet_down: AtomicU32,
        packet_up: AtomicU32,
        text_units: AtomicU32,
        unexpected: AtomicU32,
        overflow: AtomicBool,
    }

    impl InputOracle {
        fn new(mode: InputOracleMode, target: HWND) -> Self {
            Self {
                mode,
                target: handle_value(target),
                keys: AtomicU32::new(0),
                packet_down: AtomicU32::new(0),
                packet_up: AtomicU32::new(0),
                text_units: AtomicU32::new(0),
                unexpected: AtomicU32::new(0),
                overflow: AtomicBool::new(false),
            }
        }

        fn owns(&self, hwnd: HWND) -> bool {
            handle_value(hwnd) == self.target
                && TARGET.load(Ordering::Acquire) == self.target
                && !REPLACEMENT_ACTIVE.load(Ordering::Acquire)
        }

        fn increment(&self, counter: &AtomicU32) -> u32 {
            let ceiling = u32::try_from(MAX_RECORDED_EVENTS).unwrap_or(u32::MAX);
            match counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < ceiling).then(|| value + 1)
            }) {
                Ok(previous) => previous,
                Err(previous) => {
                    self.overflow.store(true, Ordering::Release);
                    previous
                }
            }
        }

        fn observe(&self, message: u32, wparam: WPARAM, lparam: LPARAM) {
            let single = lparam.0.cast_unsigned() & 0xffff == 1;
            let expected = match message {
                WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                    if wparam.0 == usize::from(VK_PACKET.0) {
                        let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
                        let index = self.increment(if down {
                            &self.packet_down
                        } else {
                            &self.packet_up
                        });
                        let paired = if down {
                            index == self.packet_up.load(Ordering::Acquire)
                        } else {
                            self.packet_down.load(Ordering::Acquire) == index.saturating_add(1)
                        };
                        self.mode == InputOracleMode::System
                            && self.keys.load(Ordering::Acquire) == 4
                            && matches!(message, WM_KEYDOWN | WM_KEYUP)
                            && index < 3
                            && paired
                            && single
                    } else {
                        let index = self.increment(&self.keys);
                        let mut key = u16::try_from(wparam.0).ok();
                        // Only the generic Shift modifier admits native side-specific forms.
                        if self.mode == InputOracleMode::System
                            && (key == Some(VK_LSHIFT.0) || key == Some(VK_RSHIFT.0))
                        {
                            key = Some(VK_SHIFT.0);
                        }
                        INPUT_ORACLE_KEYS
                            .get(usize::try_from(index).expect("bounded oracle index"))
                            .is_some_and(|&(expected_message, expected_key)| {
                                message == expected_message && key == Some(expected_key)
                            })
                            && self.packet_down.load(Ordering::Acquire) == 0
                            && self.packet_up.load(Ordering::Acquire) == 0
                            && self.text_units.load(Ordering::Acquire) == 0
                            && single
                    }
                }
                WM_CHAR => {
                    let index = self.increment(&self.text_units);
                    INPUT_ORACLE_TEXT
                        .get(usize::try_from(index).expect("bounded oracle index"))
                        .is_some_and(|&unit| wparam.0 == usize::from(unit))
                        && self.keys.load(Ordering::Acquire) == 4
                        && (self.mode == InputOracleMode::WindowMessage
                            || self.packet_down.load(Ordering::Acquire) > index)
                        && single
                }
                WM_KEYFIRST..=WM_KEYLAST
                | WM_INPUT
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_XBUTTONDOWN
                | WM_XBUTTONUP
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL => false,
                _ => return,
            };
            if !expected {
                self.increment(&self.unexpected);
            }
        }

        fn print_report(&self) {
            let keys = self.keys.load(Ordering::Acquire);
            let packet_down = self.packet_down.load(Ordering::Acquire);
            let packet_up = self.packet_up.load(Ordering::Acquire);
            let text_units = self.text_units.load(Ordering::Acquire);
            let unexpected = self.unexpected.load(Ordering::Acquire);
            let overflow = self.overflow.load(Ordering::Acquire);
            let key_match =
                keys == 4 && unexpected == 0 && !overflow && self.owns(load_handle(&TARGET));
            let expected_packets = if self.mode == InputOracleMode::System {
                3
            } else {
                0
            };
            let text_match = key_match
                && text_units == 3
                && packet_down == expected_packets
                && packet_up == expected_packets;
            print_line(&format!(
                "input-oracle keys={keys} key-match={} packet-down={packet_down} packet-up={packet_up} text-units={text_units} text-match={} unexpected={unexpected} overflow={}",
                u8::from(key_match),
                u8::from(text_match),
                u8::from(overflow),
            ));
        }
    }

    #[derive(Debug)]
    pub(super) enum RunError {
        Startup(Failure),
        Runtime(Error),
    }

    impl From<Failure> for RunError {
        fn from(failure: Failure) -> Self {
            Self::Startup(failure)
        }
    }

    pub(super) fn report_error(error: &RunError) {
        match error {
            RunError::Startup(failure) => print_line(&failure.to_string()),
            RunError::Runtime(error) => eprintln!(
                "ordinary input fixture runtime failed: status=0x{:08X}",
                error.code().0.cast_unsigned()
            ),
        }
    }

    fn windows_status(error: Error) -> Status {
        Status::WindowsHresult(error.code().0.cast_unsigned())
    }

    fn windows_failure(stage: Stage, error: Error, context: Context) -> Failure {
        Failure::new(stage, windows_status(error), context)
    }

    pub(super) struct Options {
        token: String,
        activate: bool,
        fail_request: bool,
        dpi: DpiMode,
        coordinate_oracle: bool,
        input_oracle: Option<InputOracleMode>,
    }

    pub(super) fn options() -> Result<Options, String> {
        let mut token = None;
        let mut activate = false;
        let mut fail_request = false;
        let mut dpi = None;
        let mut coordinate_oracle = false;
        let mut input_oracle = None;
        let mut arguments = std::env::args().skip(1);
        while let Some(argument) = arguments.next() {
            if argument == "--input-oracle" || argument.starts_with("--input-oracle=") {
                if input_oracle.is_some() {
                    return Err("--input-oracle may be supplied only once".to_owned());
                }
                let separate;
                let value = if let Some(value) = argument.strip_prefix("--input-oracle=") {
                    value
                } else {
                    separate = arguments.next().ok_or_else(|| {
                        "--input-oracle requires window-message or system".to_owned()
                    })?;
                    &separate
                };
                input_oracle = Some(match value {
                    "window-message" => InputOracleMode::WindowMessage,
                    "system" => InputOracleMode::System,
                    _ => {
                        return Err("--input-oracle requires window-message or system".to_owned());
                    }
                });
                continue;
            }
            if argument == "--coordinate-oracle" {
                if coordinate_oracle {
                    return Err("--coordinate-oracle may be supplied only once".to_owned());
                }
                coordinate_oracle = true;
                continue;
            }
            if let Some(value) = argument.strip_prefix("--dpi=") {
                if dpi.replace(value.parse::<DpiMode>()?).is_some() {
                    return Err("--dpi may be supplied only once".to_owned());
                }
                continue;
            }
            if argument == "--activate" {
                if activate {
                    return Err("--activate may be supplied only once".to_owned());
                }
                activate = true;
                continue;
            }
            if argument == "--fail-stage=foreground-request" {
                if fail_request {
                    return Err("--fail-stage may be supplied only once".to_owned());
                }
                fail_request = true;
                continue;
            }
            let Some(value) = argument.strip_prefix("--title-token=") else {
                return Err(format!(
                    "unknown argument `{argument}`; expected --title-token, --dpi, --coordinate-oracle, --input-oracle, --activate, or --fail-stage=foreground-request"
                ));
            };
            if value.is_empty() || value.chars().count() > 64 {
                return Err("title token must contain 1..=64 characters".to_owned());
            }
            if token.replace(value.to_owned()).is_some() {
                return Err("--title-token may be supplied only once".to_owned());
            }
        }
        if fail_request && !activate {
            return Err("--fail-stage requires --activate".to_owned());
        }
        Ok(Options {
            token: token.unwrap_or_else(|| std::process::id().to_string()),
            activate,
            fail_request,
            dpi: dpi.unwrap_or(DpiMode::PerMonitorV2),
            coordinate_oracle,
            input_oracle,
        })
    }

    fn activate_for_fixture_setup(
        window: HWND,
        context: Context,
        fail_request: bool,
    ) -> Result<(Context, ActivationPath), Failure> {
        // The matrix owns one unrelated foreground fixture. Attachment follows
        // only a refused direct request and is balanced before readiness.
        // SAFETY: window is live. Attachment closures run only when both thread
        // identifiers name distinct desktop GUI threads with message queues.
        unsafe {
            let current_thread = GetCurrentThreadId();
            let foreground = GetForegroundWindow();
            let foreground_thread = GetWindowThreadProcessId(foreground, None);
            let foreground_state = if foreground == HWND::default() || foreground_thread == 0 {
                Foreground::Absent
            } else if foreground_thread == current_thread {
                Foreground::SelfThread
            } else {
                Foreground::Present
            };
            let may_attach = foreground_thread != 0 && foreground_thread != current_thread;
            let context = context.with_foreground(foreground_state);
            let _was_visible = ShowWindow(window, SW_SHOW);
            // The diagnostic injection refuses every direct or attached request.
            activate_with_fallback(
                context,
                may_attach,
                || {
                    SetLastError(WIN32_ERROR(0));
                    if !fail_request && SetForegroundWindow(window).as_bool() {
                        Ok(())
                    } else {
                        Err(Status::Boolean {
                            ambient_win32: GetLastError().0,
                        })
                    }
                },
                || {
                    AttachThreadInput(current_thread, foreground_thread, true)
                        .ok()
                        .map_err(windows_status)
                },
                || {
                    AttachThreadInput(current_thread, foreground_thread, false)
                        .ok()
                        .map_err(windows_status)
                },
            )
        }
    }

    pub(super) fn run(options: Options) -> Result<(), RunError> {
        let translate = options.input_oracle == Some(InputOracleMode::System);
        let activation = prepare(options)?;
        print_line(&ready_record(
            ORDINARY_CLASS_NAME,
            &ordinary_title(),
            MAX_RECORDED_EVENTS,
            activation,
        ));

        let mut message = MSG::default();
        loop {
            // SAFETY: message is writable and this thread owns every fixture window.
            let status = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
            if status.0 == -1 {
                return Err(RunError::Runtime(Error::from_thread()));
            }
            if status.0 == 0 {
                break;
            }
            // Only System input needs VK_PACKET translated; WM posts text explicitly.
            // SAFETY: GetMessageW initialized the scalar message structure.
            unsafe {
                if translate {
                    let _translated = TranslateMessage(&raw const message);
                }
                DispatchMessageW(&raw const message);
            }
        }
        Ok(())
    }

    fn observe_dpi_after_failure() -> DpiAfterFailure {
        // SAFETY: the process-default set has already failed; these calls only
        // classify the resulting DPI context and cannot confound that failure.
        unsafe {
            let context = GetThreadDpiAwarenessContext();
            if AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .as_bool()
            {
                return DpiAfterFailure::PerMonitorV2;
            }
            let awareness = GetAwarenessFromDpiAwarenessContext(context);
            if awareness == DPI_AWARENESS_UNAWARE {
                DpiAfterFailure::Unaware
            } else if awareness == DPI_AWARENESS_SYSTEM_AWARE {
                DpiAfterFailure::System
            } else if awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
                DpiAfterFailure::PerMonitor
            } else {
                DpiAfterFailure::Unknown
            }
        }
    }

    fn prepare(options: Options) -> Result<Option<ActivationPath>, Failure> {
        let Options {
            token,
            activate,
            fail_request,
            dpi,
            coordinate_oracle,
            input_oracle,
        } = options;
        let mut context = Context::new();
        // SAFETY: this is the fixture's first DPI-dependent call.
        let dpi_result = unsafe { SetProcessDpiAwarenessContext(dpi.context()) };
        if let Err(error) = dpi_result {
            context = context.with_dpi_after_failure(observe_dpi_after_failure());
            return Err(windows_failure(Stage::DpiAwareness, error, context));
        }
        TITLE_TOKEN.set(token).expect("title token set once");
        COORDINATE_ORACLE.store(coordinate_oracle, Ordering::Release);
        if coordinate_oracle {
            print_line(&format!("coordinate-dpi target={}", dpi.label()));
        }
        ANIMATED.store(false, Ordering::Release);
        let class_name = wide(ORDINARY_CLASS_NAME);
        // SAFETY: null requests the current executable module.
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| windows_failure(Stage::ModuleHandle, error, context))?;
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).expect("WNDCLASSEXW fits u32"),
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..WNDCLASSEXW::default()
        };
        // SAFETY: registration copies the class name, and every field is initialized.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(windows_failure(
                Stage::ClassRegistration,
                Error::from_thread(),
                context,
            ));
        }

        let target = create_top_level(&ordinary_title(), 120, 120)
            .map_err(|error| windows_failure(Stage::WindowCreateTarget, error, context))?;
        let game = create_top_level(&role_title("Game"), 1_200, 120)
            .map_err(|error| windows_failure(Stage::WindowCreateGame, error, context))?;
        let sibling = create_top_level(&role_title("Sibling"), 800, 120)
            .map_err(|error| windows_failure(Stage::WindowCreateSibling, error, context))?;
        let child = create_child(sibling, &role_title("Child"))
            .map_err(|error| windows_failure(Stage::WindowCreateChild, error, context))?;
        let foreground = create_top_level(&role_title("Foreground"), 800, 520)
            .map_err(|error| windows_failure(Stage::WindowCreateForeground, error, context))?;
        let raw = create_top_level(&role_title("Raw"), 120, 560)
            .map_err(|error| windows_failure(Stage::WindowCreateRaw, error, context))?;
        let state = create_top_level(&role_title("State"), 460, 560)
            .map_err(|error| windows_failure(Stage::WindowCreateState, error, context))?;

        TARGET.store(handle_value(target), Ordering::Release);
        GAME.store(handle_value(game), Ordering::Release);
        SIBLING.store(handle_value(sibling), Ordering::Release);
        CHILD.store(handle_value(child), Ordering::Release);
        FOREGROUND.store(handle_value(foreground), Ordering::Release);
        RAW.store(handle_value(raw), Ordering::Release);
        STATE.store(handle_value(state), Ordering::Release);
        if let Some(mode) = input_oracle {
            assert!(
                INPUT_ORACLE.set(InputOracle::new(mode, target)).is_ok(),
                "input oracle set once"
            );
        }

        for window in [target, game, sibling, foreground, raw, state] {
            // SAFETY: each handle is a live top-level window owned by this thread.
            unsafe {
                let _was_visible = ShowWindow(window, SW_SHOWNOACTIVATE);
            }
        }
        let activation = if activate {
            let (next_context, path) = activate_for_fixture_setup(target, context, fail_request)?;
            context = next_context;
            Some(path)
        } else {
            None
        };

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
            )
            .map_err(|error| windows_failure(Stage::RawInputRegistration, error, context))?;
        }
        // SAFETY: the state window is live on this thread; a null callback posts WM_TIMER.
        if unsafe { SetTimer(Some(state), STATE_TIMER, STATE_POLL_INTERVAL_MS, None) } == 0 {
            return Err(windows_failure(
                Stage::StateTimer,
                Error::from_thread(),
                context,
            ));
        }
        Ok(activation)
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if let Some(oracle) = INPUT_ORACLE.get()
            && oracle.owns(hwnd)
        {
            oracle.observe(message, wparam, lparam);
        }
        match message {
            CONTROL_REPORT => {
                if wparam.0 == 2
                    && let Some(oracle) = INPUT_ORACLE.get()
                {
                    oracle.print_report();
                    return LRESULT(0);
                }
                if wparam.0 == 1 && COORDINATE_ORACLE.load(Ordering::Acquire) {
                    print_coordinate_oracle(hwnd);
                }
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
                let updated = {
                    let Some(_dpi) = DpiScope::enter(DpiMode::PerMonitorV2) else {
                        print_line("control geometry=failed");
                        return LRESULT(0);
                    };
                    // SAFETY: this thread owns hwnd; control coordinates are physical.
                    unsafe {
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
                    .is_ok()
                };
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
            CONTROL_SET_VISUAL_ABSENT | CONTROL_SET_VISUAL_VISIBLE => {
                let Some(command) = visual_command_for_control(message, wparam.0) else {
                    print_line("control visual-state=failed");
                    return LRESULT(0);
                };
                WATCH_VISUAL_STATE.store(command.packed(), Ordering::Release);
                if repaint_target() {
                    print_line(&command.acknowledgement());
                } else {
                    print_line("control visual-state=failed");
                }
                LRESULT(0)
            }
            CONTROL_TRANSITION_VISUAL => {
                print_line(if pulse_target_paint() {
                    VISUAL_TRANSITION_ACKNOWLEDGEMENT
                } else {
                    "control visual-transition=failed"
                });
                LRESULT(0)
            }
            CONTROL_DESTROY_TARGET => {
                // SAFETY: control is dispatched only to the live retained target.
                let destroyed = unsafe { DestroyWindow(hwnd) }.is_ok();
                print_line(if destroyed {
                    TARGET_LOSS_ACKNOWLEDGEMENT
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
            WM_MOUSEMOVE => observe_pointer(hwnd, "pointer-move", lparam),
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                observe_pointer(hwnd, "button-down", lparam)
            }
            WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
                observe_pointer(hwnd, "button-up", lparam)
            }
            WM_MOUSEWHEEL => observe_pointer(hwnd, "vertical-wheel", lparam),
            WM_MOUSEHWHEEL => observe_pointer(hwnd, "horizontal-wheel", lparam),
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

    fn observe_pointer(hwnd: HWND, family: &'static str, lparam: LPARAM) -> LRESULT {
        let result = observe_legacy(hwnd, family);
        let (role, counter) = role(hwnd);
        if COORDINATE_ORACLE.load(Ordering::Acquire) {
            let bits = lparam.0.cast_unsigned();
            let x = u16::try_from(bits & 0xffff)
                .expect("masked x")
                .cast_signed();
            let y = u16::try_from((bits >> 16) & 0xffff)
                .expect("masked y")
                .cast_signed();
            print_line(&format!(
                "coordinate role={role} family={family} sequence={} x={x} y={y}",
                counter.load(Ordering::Acquire),
            ));
        }
        result
    }

    fn print_coordinate_oracle(hwnd: HWND) {
        let mut bounds = RECT::default();
        // SAFETY: this thread owns hwnd and bounds is writable.
        if unsafe { GetClientRect(hwnd, &raw mut bounds) }.is_err()
            || bounds.right < 4
            || bounds.bottom < 4
        {
            print_line("coordinate-oracle failed=bounds");
            return;
        }
        let client = POINT {
            x: bounds.right / 2,
            y: bounds.bottom / 2,
        };
        let span = bounds.right.min(bounds.bottom) / 4;
        let mut screen = client;
        let mut far = POINT {
            x: client.x + span,
            y: client.y + span,
        };
        // SAFETY: both points are writable and this thread owns the target.
        if !unsafe {
            ClientToScreen(hwnd, &raw mut screen).as_bool()
                && ClientToScreen(hwnd, &raw mut far).as_bool()
        } {
            print_line("coordinate-oracle failed=client");
            return;
        }
        let mut physical = screen;
        // SAFETY: this fixture owns hwnd and both points remain writable.
        if !unsafe {
            LogicalToPhysicalPointForPerMonitorDPI(Some(hwnd), &raw mut physical).as_bool()
                && LogicalToPhysicalPointForPerMonitorDPI(Some(hwnd), &raw mut far).as_bool()
        } {
            print_line("coordinate-oracle failed=physical");
            return;
        }
        print_line(&format!(
            "coordinate-oracle client-x={} client-y={} screen-x={} screen-y={} physical-x={} physical-y={} span-x={} span-y={} logical-span={span}",
            client.x,
            client.y,
            screen.x,
            screen.y,
            physical.x,
            physical.y,
            far.x - physical.x,
            far.y - physical.y,
        ));
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

    fn repaint_target() -> bool {
        let target = load_handle(&TARGET);
        // SAFETY: the retained target is owned by this GUI thread.
        unsafe { InvalidateRect(Some(target), None, true) }.as_bool()
            && unsafe { UpdateWindow(target) }.as_bool()
    }

    fn pulse_target_paint() -> bool {
        ANIMATED.fetch_xor(true, Ordering::AcqRel);
        repaint_target()
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
            let is_target = handle_value(hwnd) == TARGET.load(Ordering::Acquire);
            let fill = if is_target && ANIMATED.load(Ordering::Acquire) {
                BENCHMARK_FILL_RGB
            } else {
                FILL_RGB
            };
            fill_solid_rect(device, &client, fill);
            if is_target
                && let Some(command) =
                    FixtureVisualCommand::from_packed(WATCH_VISUAL_STATE.load(Ordering::Acquire))
            {
                paint_watch_visual(device, command);
            }
        }
        // SAFETY: balances BeginPaint for this WM_PAINT dispatch.
        let _ended = unsafe { EndPaint(hwnd, &raw const paint) };
    }

    fn paint_watch_visual(device: HDC, command: FixtureVisualCommand) {
        // SAFETY: both brushes are process-owned and deleted before this function returns.
        let primary = unsafe { CreateSolidBrush(colorref(WATCH_MARKER_PRIMARY_RGB)) };
        // SAFETY: same ownership rule as `primary`.
        let secondary = unsafe { CreateSolidBrush(colorref(WATCH_MARKER_SECONDARY_RGB)) };
        if command.state().marker_is_visible() {
            let marker = RECT {
                left: WATCH_MARKER_X,
                top: WATCH_MARKER_Y,
                right: WATCH_MARKER_X + WATCH_MARKER_WIDTH,
                bottom: WATCH_MARKER_Y + WATCH_MARKER_HEIGHT,
            };
            let top_middle = RECT {
                left: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE,
                top: WATCH_MARKER_Y,
                right: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE * 2,
                bottom: WATCH_MARKER_Y + WATCH_MARKER_CELL_SIZE,
            };
            let bottom_left = RECT {
                left: WATCH_MARKER_X,
                top: WATCH_MARKER_Y + WATCH_MARKER_CELL_SIZE,
                right: WATCH_MARKER_X + WATCH_MARKER_CELL_SIZE,
                bottom: WATCH_MARKER_Y + WATCH_MARKER_HEIGHT,
            };
            fill_rect_with_brush(device, &marker, primary);
            fill_rect_with_brush(device, &top_middle, secondary);
            fill_rect_with_brush(device, &bottom_left, secondary);
        }

        for index in 0..WATCH_TOKEN_CELL_COUNT {
            let column = i32::try_from(index % WATCH_TOKEN_GRID_WIDTH)
                .expect("visual-token column fits i32");
            let row =
                i32::try_from(index / WATCH_TOKEN_GRID_WIDTH).expect("visual-token row fits i32");
            let left = WATCH_TOKEN_X + column * WATCH_TOKEN_CELL_SIZE;
            let top = WATCH_TOKEN_Y + row * WATCH_TOKEN_CELL_SIZE;
            let cell = RECT {
                left,
                top,
                right: left + WATCH_TOKEN_CELL_SIZE,
                bottom: top + WATCH_TOKEN_CELL_SIZE,
            };
            let brush = if visual_token_cell(command, index)
                .expect("index is bounded by WATCH_TOKEN_CELL_COUNT")
            {
                primary
            } else {
                secondary
            };
            fill_rect_with_brush(device, &cell, brush);
        }

        // SAFETY: neither brush remains selected after the FillRect calls above.
        let _primary_deleted = unsafe { DeleteObject(HGDIOBJ(primary.0)) };
        // SAFETY: same lifetime rule as `primary`.
        let _secondary_deleted = unsafe { DeleteObject(HGDIOBJ(secondary.0)) };
    }

    fn fill_rect_with_brush(
        device: HDC,
        rectangle: &RECT,
        brush: windows::Win32::Graphics::Gdi::HBRUSH,
    ) {
        // SAFETY: device, rectangle, and process-owned brush are live for this paint call.
        let _filled = unsafe { FillRect(device, rectangle, brush) };
    }

    fn fill_solid_rect(device: HDC, rectangle: &RECT, rgb: u32) {
        // SAFETY: creating, using, and deleting this process-owned brush is paired.
        let brush = unsafe { CreateSolidBrush(colorref(rgb)) };
        // SAFETY: device, rectangle, and brush are live for this paint call.
        let _filled = unsafe { FillRect(device, rectangle, brush) };
        // SAFETY: the brush is no longer selected or needed after FillRect.
        let _deleted = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
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

    fn create_top_level(title: &str, x: i32, y: i32) -> WindowsResult<HWND> {
        create_window(title, x, y, 360, 240, None, WS_OVERLAPPEDWINDOW)
    }

    fn create_child(parent: HWND, title: &str) -> WindowsResult<HWND> {
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
    ) -> WindowsResult<HWND> {
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
