// Private OCR watcher verification apparatus; not a product adapter or input route.
// Build only with the existing x64 MSVC compiler and Windows SDK. No third-party library.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <dwmapi.h>
#include <array>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

namespace {
constexpr wchar_t ClassName[] = L"MadoPilot.Private.OcrTextWatch.Windows.1";
constexpr DWORD RunMilliseconds = 300000;
constexpr size_t ImageBytes = 960 * 540 * 4;

struct Fixture {
    HWND window = nullptr;
    HWND foreground = nullptr;
    std::array<unsigned char, 16> nonce{};
    std::vector<unsigned char> image;
    unsigned sequence = 0, state = 0, paints = 0;
    int width = 960, height = 576;
    bool failed = false;
};

LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* fixture = reinterpret_cast<Fixture*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
        fixture = static_cast<Fixture*>(create->lpCreateParams);
        SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(fixture));
    }
    if (message == WM_MOUSEACTIVATE) return MA_NOACTIVATE;
    if (message == WM_ACTIVATE && fixture && LOWORD(wparam) != WA_INACTIVE)
        fixture->failed = true;
    if (message == WM_PAINT && fixture) {
        PAINTSTRUCT paint{};
        HDC dc = BeginPaint(window, &paint);
        RECT bounds{};
        if (!dc || !GetClientRect(window, &bounds)) fixture->failed = true;
        if (dc) {
            HBRUSH brush = reinterpret_cast<HBRUSH>(GetStockObject(DC_BRUSH));
            SetDCBrushColor(dc, RGB(255, 255, 255));
            if (!FillRect(dc, &bounds, brush)) fixture->failed = true;
            if (fixture->state == 1) {
                BITMAPINFO info{};
                info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
                info.bmiHeader.biWidth = 960;
                info.bmiHeader.biHeight = -540;
                info.bmiHeader.biPlanes = 1;
                info.bmiHeader.biBitCount = 32;
                info.bmiHeader.biCompression = BI_RGB;
                if (StretchDIBits(dc, 0, 0, 960, 540, 0, 0, 960, 540,
                        fixture->image.data(), &info, DIB_RGB_COLORS, SRCCOPY) != 540)
                    fixture->failed = true;
            }
            for (unsigned cell = 0; cell < 18; ++cell) {
                unsigned char value = cell < 16 ? fixture->nonce[cell] :
                    static_cast<unsigned char>(cell == 16 ? fixture->sequence : fixture->state);
                SetDCBrushColor(dc, RGB(value, value, value));
                RECT marker{static_cast<LONG>(cell * 8), 544,
                    static_cast<LONG>(cell * 8 + 8), 552};
                if (!FillRect(dc, &marker, brush)) fixture->failed = true;
            }
            ++fixture->paints;
        }
        EndPaint(window, &paint);
        return 0;
    }
    if (message == WM_CLOSE) {
        if (fixture) fixture->failed = true; // Only the private destroy command is authorized.
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

bool acknowledge(Fixture& fixture) {
    if (fixture.state < 2) {
        const auto before = fixture.paints;
        if (!RedrawWindow(fixture.window, nullptr, nullptr,
                RDW_INVALIDATE | RDW_UPDATENOW | RDW_ERASE) ||
            fixture.paints <= before || !GdiFlush() || FAILED(DwmFlush())) return false;
        RECT bounds{};
        if (!GetClientRect(fixture.window, &bounds) || bounds.right != fixture.width ||
            bounds.bottom != fixture.height) return false;
    }
    if (fixture.failed || GetForegroundWindow() != fixture.foreground) return false;
    char line[192];
    const int size = sprintf_s(line, "ACK %u %lu %llu %d %d %u %u\n", fixture.sequence,
        GetCurrentProcessId(), static_cast<unsigned long long>(
            reinterpret_cast<ULONG_PTR>(fixture.window)), fixture.width, fixture.height,
        fixture.state, fixture.paints);
    DWORD written = 0;
    return size > 0 && WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), line,
        static_cast<DWORD>(size), &written, nullptr) && written == static_cast<DWORD>(size);
}

int fixture_main(int argc, wchar_t** argv) {
    if (argc != 4 || std::wstring(argv[1]) != L"--fixture") return 2;
    Fixture fixture;
    const std::wstring token(argv[3]);
    if (token.size() != 32) return 2;
    for (size_t i = 0; i < 32; ++i) {
        const auto c = token[i];
        const int nibble = c >= L'0' && c <= L'9' ? c - L'0' :
            c >= L'a' && c <= L'f' ? c - L'a' + 10 : -1;
        if (nibble < 0) return 2;
        fixture.nonce[i / 2] |= static_cast<unsigned char>(nibble << (i % 2 ? 0 : 4));
    }
    std::ifstream image(std::filesystem::path(argv[2]), std::ios::binary | std::ios::ate);
    if (!image || image.tellg() != static_cast<std::streamoff>(ImageBytes)) return 2;
    fixture.image.resize(ImageBytes);
    image.seekg(0);
    if (!image.read(reinterpret_cast<char*>(fixture.image.data()), ImageBytes)) return 2;
    if (!SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)) return 2;
    fixture.foreground = GetForegroundWindow();
    if (!fixture.foreground) return 2;
    MONITORINFO monitor{sizeof(MONITORINFO)};
    const POINT origin{0, 0};
    if (!GetMonitorInfoW(MonitorFromPoint(origin, MONITOR_DEFAULTTOPRIMARY), &monitor) ||
        monitor.rcWork.right - monitor.rcWork.left < 1088 ||
        monitor.rcWork.bottom - monitor.rcWork.top < 704) return 2;
    WNDCLASSW type{};
    type.lpfnWndProc = window_proc;
    type.hInstance = GetModuleHandleW(nullptr);
    type.lpszClassName = ClassName;
    if (!RegisterClassW(&type)) return 2;
    const std::wstring title = L"MadoPilot OCR private [" + token + L"]";
    fixture.window = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, ClassName,
        title.c_str(), WS_POPUP, monitor.rcWork.left + 32, monitor.rcWork.top + 32,
        fixture.width, fixture.height, nullptr, nullptr, type.hInstance, &fixture);
    if (!fixture.window) { UnregisterClassW(ClassName, type.hInstance); return 2; }
    const DWM_WINDOW_CORNER_PREFERENCE corners = DWMWCP_DONOTROUND;
    bool ok = SUCCEEDED(DwmSetWindowAttribute(fixture.window, DWMWA_WINDOW_CORNER_PREFERENCE,
        &corners, sizeof(corners)));
    ShowWindow(fixture.window, SW_SHOWNOACTIVATE);
    ok = ok && acknowledge(fixture);
    const ULONGLONG until = GetTickCount64() + RunMilliseconds;
    std::string command;
    command.reserve(32);
    bool quit = false;
    while (ok && !quit && GetTickCount64() < until) {
        MSG message{};
        while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if (fixture.failed || GetForegroundWindow() != fixture.foreground) { ok = false; break; }
        DWORD available = 0;
        if (!PeekNamedPipe(GetStdHandle(STD_INPUT_HANDLE), nullptr, 0, nullptr, &available, nullptr)) {
            ok = false; break;
        }
        if (available) {
            char byte = 0;
            DWORD count = 0;
            if (!ReadFile(GetStdHandle(STD_INPUT_HANDLE), &byte, 1, &count, nullptr) || count != 1) {
                ok = false; break;
            }
            if (byte != '\n') {
                if (command.size() >= 31 || byte < ' ' || byte > '~') { ok = false; break; }
                command += byte;
                continue;
            }
            ++fixture.sequence;
            if (fixture.sequence > 16) { ok = false; break; }
            if (command == "quit") {
                if (IsWindow(fixture.window) && !DestroyWindow(fixture.window)) { ok = false; break; }
                fixture.state = 2;
                quit = true;
            } else if (fixture.state == 2) { ok = false; break; }
            else if (command == "blank") fixture.state = 0;
            else if (command == "show") fixture.state = 1;
            else if (command == "tick") { /* Only the out-of-ROI sequence marker changes. */ }
            else if (command == "resize") {
                fixture.width = 1024;
                fixture.height = 640;
                if (!SetWindowPos(fixture.window, nullptr, 0, 0, fixture.width, fixture.height,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE)) { ok = false; break; }
            } else if (command == "destroy") {
                if (!DestroyWindow(fixture.window) || IsWindow(fixture.window)) { ok = false; break; }
                fixture.state = 2;
            } else { ok = false; break; }
            command.clear();
            ok = acknowledge(fixture);
        } else Sleep(10);
    }
    if (IsWindow(fixture.window) && !DestroyWindow(fixture.window)) ok = false;
    if (!UnregisterClassW(ClassName, type.hInstance)) ok = false;
    return ok && quit ? 0 : 1;
}
} // namespace

int wmain(int argc, wchar_t** argv) {
    try {
        return fixture_main(argc, argv);
    } catch (...) {
        // No raw exceptions, paths, text, or native identifiers on ordinary output.
        return 1;
    }
}
