// Private pacing apparatus; no capture, input, activation, or permission APIs.
// Build: cl /nologo /std:c++17 /EHsc /W4 fixture.cpp /link user32.lib gdi32.lib dwmapi.lib advapi32.lib psapi.lib
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <aclapi.h>
#include <dwmapi.h>
#include <psapi.h>
#include <algorithm>
#include <array>
#include <cstdarg>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <cwchar>
#include <limits>
#include <string>
#include <vector>

namespace {
constexpr wchar_t ClassName[] = L"MadoPilot.Private.CapturePacing.Windows.1";
constexpr DWORD FuseMilliseconds = 180000;
constexpr uint64_t AnimationNanoseconds = 16000000;
constexpr size_t ImageBytes = 960 * 540 * 4;
constexpr size_t SurfacePixels = 1040 * 640;
constexpr size_t SampleLimit = 4096;
constexpr size_t JsonLimit = 131072;

enum class Error {
    None, Arguments, Root, File, Command, Sequence, Foreground, Image, Render,
    Geometry, Clock, Output, Closed, Native, Fuse, Cleanup, Exception
};

const char* reason(Error error) {
    switch (error) {
    case Error::None: return "fixture-ok";
    case Error::Arguments: return "fixture-arguments-invalid";
    case Error::Root: return "fixture-root-invalid";
    case Error::File: return "fixture-file-invalid";
    case Error::Command: return "fixture-command-invalid";
    case Error::Sequence: return "fixture-sequence-invalid";
    case Error::Foreground: return "fixture-foreground-changed";
    case Error::Image: return "fixture-image-invalid";
    case Error::Render: return "fixture-render-failed";
    case Error::Geometry: return "fixture-geometry-invalid";
    case Error::Clock: return "fixture-clock-failed";
    case Error::Output: return "fixture-output-failed";
    case Error::Closed: return "fixture-window-closed";
    case Error::Native: return "fixture-native-failed";
    case Error::Fuse: return "fixture-fuse-expired";
    case Error::Cleanup: return "fixture-cleanup-failed";
    case Error::Exception: return "fixture-exception";
    }
    return "fixture-native-failed";
}

struct Handle {
    HANDLE value = INVALID_HANDLE_VALUE;
    Handle() = default;
    explicit Handle(HANDLE handle) : value(handle) {}
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    ~Handle() { close(); }
    bool valid() const { return value != INVALID_HANDLE_VALUE && value != nullptr; }
    bool close() {
        if (!valid()) return true;
        HANDLE old = value;
        value = INVALID_HANDLE_VALUE;
        return CloseHandle(old) != FALSE;
    }
};

enum FileName : size_t { Command, Stop, Ack, AckTemp, Metrics, MetricsTemp, Failure, FailureTemp, FileCount };
constexpr const wchar_t* FileNames[FileCount] = {
    L"command", L"stop", L"ack", L"ack.tmp", L"fixture-metrics.json",
    L"fixture-metrics.json.tmp", L"fixture-error", L"fixture-error.tmp"
};

struct Control {
    Handle root;
    std::array<std::wstring, FileCount> paths;
    alignas(SID) unsigned char user[SECURITY_MAX_SID_SIZE]{};
    alignas(SID) unsigned char system[SECURITY_MAX_SID_SIZE]{};
    alignas(SID) unsigned char administrators[SECURITY_MAX_SID_SIZE]{};
    alignas(SID) unsigned char ownerRights[SECURITY_MAX_SID_SIZE]{};
    alignas(ACL) unsigned char aclBytes[sizeof(ACL) + sizeof(ACCESS_ALLOWED_ACE) + SECURITY_MAX_SID_SIZE]{};
    SECURITY_DESCRIPTOR descriptor{};
    SECURITY_ATTRIBUTES attributes{sizeof(SECURITY_ATTRIBUTES), &descriptor, FALSE};
    bool ready = false;

    bool private_owner(HANDLE file) const {
        PSID owner = nullptr;
        PACL dacl = nullptr;
        PSECURITY_DESCRIPTOR security = nullptr;
        const DWORD status = GetSecurityInfo(file, SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &owner, nullptr, &dacl, nullptr, &security);
        if (status != ERROR_SUCCESS) return false;
        bool valid = owner && EqualSid(owner, const_cast<unsigned char*>(user)) && dacl;
        bool ownAccess = false;
        if (valid) {
            for (DWORD index = 0; index < dacl->AceCount; ++index) {
                void* entry = nullptr;
                if (!GetAce(dacl, index, &entry)) { valid = false; break; }
                const auto* header = static_cast<const ACE_HEADER*>(entry);
                if (header->AceType == ACCESS_DENIED_ACE_TYPE) continue;
                if (header->AceType != ACCESS_ALLOWED_ACE_TYPE ||
                    header->AceSize < sizeof(ACCESS_ALLOWED_ACE)) { valid = false; break; }
                const auto* ace = static_cast<const ACCESS_ALLOWED_ACE*>(entry);
                PSID sid = const_cast<DWORD*>(&ace->SidStart);
                if (!IsValidSid(sid) || GetLengthSid(sid) > header->AceSize - offsetof(ACCESS_ALLOWED_ACE, SidStart)) {
                    valid = false; break;
                }
                // OWNER RIGHTS applies to the already-verified current-user owner.
                const bool own = EqualSid(sid, const_cast<unsigned char*>(user)) != FALSE ||
                    EqualSid(sid, const_cast<unsigned char*>(ownerRights)) != FALSE;
                if (!own && !EqualSid(sid, const_cast<unsigned char*>(system)) &&
                    !EqualSid(sid, const_cast<unsigned char*>(administrators))) { valid = false; break; }
                if (own && !(header->AceFlags & INHERIT_ONLY_ACE) &&
                    ((ace->Mask & GENERIC_ALL) ||
                     (ace->Mask & (FILE_READ_DATA | FILE_WRITE_DATA)) == (FILE_READ_DATA | FILE_WRITE_DATA)))
                    ownAccess = true;
            }
        }
        LocalFree(security);
        return valid && ownAccess;
    }

    bool open(const wchar_t* path) {
        Handle token;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token.value)) return false;
        alignas(TOKEN_USER) unsigned char tokenBytes[sizeof(TOKEN_USER) + SECURITY_MAX_SID_SIZE]{};
        DWORD needed = 0;
        if (!GetTokenInformation(token.value, TokenUser, tokenBytes, sizeof(tokenBytes), &needed) ||
            !CopySid(sizeof(user), user, reinterpret_cast<TOKEN_USER*>(tokenBytes)->User.Sid)) return false;
        DWORD bytes = sizeof(system);
        if (!CreateWellKnownSid(WinLocalSystemSid, nullptr, system, &bytes)) return false;
        bytes = sizeof(administrators);
        if (!CreateWellKnownSid(WinBuiltinAdministratorsSid, nullptr, administrators, &bytes)) return false;
        bytes = sizeof(ownerRights);
        if (!CreateWellKnownSid(WinCreatorOwnerRightsSid, nullptr, ownerRights, &bytes)) return false;
        auto* acl = reinterpret_cast<ACL*>(aclBytes);
        if (!InitializeAcl(acl, sizeof(aclBytes), ACL_REVISION) ||
            !AddAccessAllowedAce(acl, ACL_REVISION, FILE_ALL_ACCESS, user) ||
            !InitializeSecurityDescriptor(&descriptor, SECURITY_DESCRIPTOR_REVISION) ||
            !SetSecurityDescriptorOwner(&descriptor, user, FALSE) ||
            !SetSecurityDescriptorDacl(&descriptor, TRUE, acl, FALSE) ||
            !SetSecurityDescriptorControl(&descriptor, SE_DACL_PROTECTED, SE_DACL_PROTECTED)) return false;
        // Denying delete sharing pins the checked directory for the process lifetime.
        root.value = CreateFileW(path, FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        BY_HANDLE_FILE_INFORMATION information{};
        if (!root.valid() || GetFileType(root.value) != FILE_TYPE_DISK ||
            !GetFileInformationByHandle(root.value, &information) ||
            !(information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) ||
            (information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) || !private_owner(root.value)) return false;
        wchar_t finalPath[2048];
        const DWORD length = GetFinalPathNameByHandleW(root.value, finalPath, 2048, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
        if (length < 7 || length >= 2048 || wcsncmp(finalPath, L"\\\\?\\", 4) != 0 ||
            finalPath[5] != L':' || finalPath[6] != L'\\' ||
            !((finalPath[4] >= L'A' && finalPath[4] <= L'Z') || (finalPath[4] >= L'a' && finalPath[4] <= L'z')))
            return false;
        const wchar_t volume[] = {finalPath[4], L':', L'\\', L'\0'};
        if (GetDriveTypeW(volume) != DRIVE_FIXED) return false;
        std::wstring prefix(finalPath, length);
        if (prefix.back() != L'\\') prefix += L'\\';
        for (size_t index = 0; index < FileCount; ++index) paths[index] = prefix + FileNames[index];
        ready = true;
        return true;
    }

    bool regular(HANDLE file, bool requirePrivate, uint64_t limit, uint64_t* size = nullptr) const {
        BY_HANDLE_FILE_INFORMATION information{};
        if (GetFileType(file) != FILE_TYPE_DISK || !GetFileInformationByHandle(file, &information) ||
            (information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) ||
            information.nNumberOfLinks > 1 || (requirePrivate && !private_owner(file))) return false;
        // An open command snapshot may have zero links after the parent's atomic replacement.
        const uint64_t length = (uint64_t(information.nFileSizeHigh) << 32) | information.nFileSizeLow;
        if (length > limit) return false;
        if (size) *size = length;
        return true;
    }

    bool stop_requested(Error& error) const {
        const DWORD stopAttributes = GetFileAttributesW(paths[Stop].c_str());
        if (stopAttributes != INVALID_FILE_ATTRIBUTES) return true; // Existence alone authorizes owned cleanup.
        if (GetLastError() != ERROR_FILE_NOT_FOUND) error = Error::File;
        return false;
    }

    bool outputs_absent() const {
        for (size_t index = Ack; index < FileCount; ++index) {
            if (GetFileAttributesW(paths[index].c_str()) != INVALID_FILE_ATTRIBUTES ||
                GetLastError() != ERROR_FILE_NOT_FOUND) return false;
        }
        return true;
    }

    bool atomic_write(FileName destination, FileName temporary, const char* bytes, size_t length) const {
        if (!ready || length > JsonLimit) return false;
        Handle existing(CreateFileW(paths[destination].c_str(), FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, nullptr));
        if (existing.valid()) {
            if (!regular(existing.value, true, JsonLimit) || !existing.close()) return false;
        } else if (GetLastError() != ERROR_FILE_NOT_FOUND) return false;
        Handle output(CreateFileW(paths[temporary].c_str(), GENERIC_WRITE | READ_CONTROL,
            FILE_SHARE_READ, const_cast<SECURITY_ATTRIBUTES*>(&attributes), CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
        if (!output.valid()) return false;
        DWORD written = 0;
        bool ok = regular(output.value, true, 0) &&
            WriteFile(output.value, bytes, static_cast<DWORD>(length), &written, nullptr) && written == length &&
            FlushFileBuffers(output.value);
        if (!output.close()) ok = false;
        if (ok) ok = MoveFileExW(paths[temporary].c_str(), paths[destination].c_str(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) != FALSE;
        if (!ok) DeleteFileW(paths[temporary].c_str());
        return ok;
    }

    void report(Error error) const {
        char row[64];
        const int length = std::snprintf(row, sizeof(row), "%s\n", reason(error));
        if (length > 0 && static_cast<size_t>(length) < sizeof(row)) {
            atomic_write(Failure, FailureTemp, row, static_cast<size_t>(length));
            std::fputs(row, stderr);
        }
    }
};

struct Fuse {
    const Control* control = nullptr;
    Handle cancel;
    Handle thread;

    static DWORD WINAPI wait(void* context) {
        auto* fuse = static_cast<Fuse*>(context);
        const DWORD result = WaitForSingleObject(fuse->cancel.value, FuseMilliseconds);
        if (result != WAIT_OBJECT_0) {
            fuse->control->report(result == WAIT_TIMEOUT ? Error::Fuse : Error::Native);
            TerminateProcess(GetCurrentProcess(), result == WAIT_TIMEOUT ? 124 : 1);
        }
        return 0;
    }
    bool start(const Control& root) {
        control = &root;
        cancel.value = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!cancel.valid()) return false;
        thread.value = CreateThread(nullptr, 0, wait, this, 0, nullptr);
        return thread.valid();
    }
    bool stop() {
        if (!thread.valid()) return true;
        if (!SetEvent(cancel.value) || WaitForSingleObject(thread.value, 5000) != WAIT_OBJECT_0) {
            control->report(Error::Cleanup);
            TerminateProcess(GetCurrentProcess(), 1);
            return false;
        }
        const bool closed = thread.close();
        return cancel.close() && closed;
    }
};

struct Clock {
    uint64_t frequency = 0;
    bool initialize() {
        LARGE_INTEGER value{};
        if (!QueryPerformanceFrequency(&value) || value.QuadPart <= 0) return false;
        frequency = static_cast<uint64_t>(value.QuadPart);
        return frequency <= std::numeric_limits<uint64_t>::max() / 1000000000;
    }
    bool now(uint64_t& result) const {
        LARGE_INTEGER value{};
        if (!frequency || !QueryPerformanceCounter(&value) || value.QuadPart < 0) return false;
        const uint64_t ticks = static_cast<uint64_t>(value.QuadPart);
        const uint64_t fraction = ((ticks % frequency) * 1000000000) / frequency;
        if (ticks / frequency > (std::numeric_limits<uint64_t>::max() - fraction) / 1000000000) return false;
        result = (ticks / frequency) * 1000000000 + fraction;
        return true;
    }
};

bool cpu_time(uint64_t& result) {
    FILETIME created{}, exited{}, kernel{}, user{};
    if (!GetProcessTimes(GetCurrentProcess(), &created, &exited, &kernel, &user)) return false;
    const uint64_t system = (uint64_t(kernel.dwHighDateTime) << 32) | kernel.dwLowDateTime;
    const uint64_t own = (uint64_t(user.dwHighDateTime) << 32) | user.dwLowDateTime;
    const uint64_t limit = std::numeric_limits<uint64_t>::max() / 100;
    if (system > limit || own > limit - system) return false;
    result = (system + own) * 100;
    return true;
}

struct Statistics {
    uint64_t wallStart = 0, cpuStart = 0, lastStart = 0, lost = 0;
    bool cpuAvailable = false, hasLastStart = false;
    std::array<uint64_t, SampleLimit> intervals{}, durations{};
    size_t intervalCount = 0, durationCount = 0;

    bool reset(const Clock& clock) {
        if (!clock.now(wallStart)) return false;
        cpuAvailable = cpu_time(cpuStart);
        hasLastStart = false;
        intervalCount = durationCount = 0;
        lost = 0;
        return true;
    }
    bool append(std::array<uint64_t, SampleLimit>& samples, size_t& count, uint64_t value) {
        if (count < SampleLimit) samples[count++] = value;
        else {
            if (lost == std::numeric_limits<uint64_t>::max()) return false;
            ++lost;
        }
        return true;
    }
    bool record(uint64_t start, uint64_t end) {
        if (end < start || (hasLastStart && start < lastStart)) return false;
        if (hasLastStart && !append(intervals, intervalCount, start - lastStart)) return false;
        lastStart = start;
        hasLastStart = true;
        return append(durations, durationCount, end - start);
    }
};

struct Json {
    std::array<char, JsonLimit> bytes;
    size_t length = 0;
    bool valid = true;
    void append(const char* format, ...) {
        if (!valid) return;
        va_list arguments;
        va_start(arguments, format);
        const int added = std::vsnprintf(bytes.data() + length, bytes.size() - length, format, arguments);
        va_end(arguments);
        if (added < 0 || static_cast<size_t>(added) >= bytes.size() - length) valid = false;
        else length += static_cast<size_t>(added);
    }
    void number(bool available, uint64_t value) {
        if (available) append("%llu", static_cast<unsigned long long>(value));
        else append("null");
    }
};

enum class Operation { Blank, Show, Pulse, Animate, Pause, Burst, Resize, ResetStats, Stats, Close, Quit };
constexpr const char* Operations[] = {
    "blank", "show", "pulse", "animate", "pause", "burst", "resize", "reset-stats", "stats", "close-window", "quit"
};

struct Fixture {
    Control control;
    Fuse fuse;
    Clock clock;
    Statistics statistics;
    HWND window = nullptr, foreground = nullptr;
    HWINEVENTHOOK foregroundHook = nullptr;
    inline static Fixture* foregroundObserverOwner = nullptr;
    uint64_t windowId = 0, nonceBits = 0, nextRender = 0;
    HINSTANCE instance = nullptr;
    std::array<char, 17> nonce{};
    std::vector<unsigned char> image;
    std::vector<uint32_t> surface;
    int surfaceWidth = 0, surfaceHeight = 0;
    uint32_t surfaceState = 0;
    uint32_t sequence = 0, counter = 0, state = 0;
    Operation lastOperation = Operation::Blank;
    unsigned burstRemaining = 0;
    int width = 960, height = 576;
    bool classRegistered = false, painted = false, animating = false, quitting = false;
    Error error = Error::None;

    void fail(Error value) { if (error == Error::None) error = value; }
    static void CALLBACK foreground_event(HWINEVENTHOOK hook, DWORD event, HWND active,
        LONG object, LONG child, DWORD thread, DWORD time) {
        (void)hook; (void)event; (void)object; (void)child; (void)thread; (void)time;
        if (foregroundObserverOwner && active != foregroundObserverOwner->foreground)
            foregroundObserverOwner->fail(Error::Foreground);
    }
    bool foreground_unchanged() {
        if (GetForegroundWindow() == foreground && foreground) return true;
        fail(Error::Foreground);
        return false;
    }
    bool geometry_valid() {
        RECT bounds{};
        if (window && GetClientRect(window, &bounds) && bounds.left == 0 && bounds.top == 0 &&
            bounds.right == width && bounds.bottom == height) return true;
        fail(Error::Geometry);
        return false;
    }
    bool compose() {
        if (!counter || state > 1 || surface.size() != SurfacePixels ||
            !((width == 960 && height == 576) || (width == 1040 && height == 640)) ||
            (state == 1 && image.size() != ImageBytes)) { fail(Error::Render); return false; }
        const bool rebuild = surfaceWidth != width || surfaceHeight != height || surfaceState != state;
        if (rebuild) {
            std::fill_n(surface.data(), static_cast<size_t>(width) * height, uint32_t{0xffffffff});
            if (state == 1) {
                for (size_t row = 0; row < 540; ++row)
                    std::memcpy(surface.data() + row * width, image.data() + row * 960 * 4, 960 * 4);
            }
        }
        // Stable frames need only new counter cells; the full retained BGRA surface stays valid.
        const unsigned first = rebuild ? 0 : 64;
        const unsigned end = rebuild ? 128 : 96;
        for (unsigned cell = first; cell < end; ++cell) {
            const bool one = cell < 64 ? ((nonceBits >> (63 - cell)) & 1) != 0 :
                cell < 96 ? ((counter >> (95 - cell)) & 1) != 0 : ((state >> (127 - cell)) & 1) != 0;
            const uint32_t color = one ? 0xffffff00 : 0xff0000ff; // Opaque yellow/blue, BGRA in memory.
            for (size_t row = 544; row < 560; ++row)
                std::fill_n(surface.data() + row * width + cell * 4, 4, color);
        }
        surfaceWidth = width;
        surfaceHeight = height;
        surfaceState = state;
        return true;
    }

    void draw(HDC dc) {
        if (!counter || !surfaceWidth || surfaceWidth != width || surfaceHeight != height) {
            fail(Error::Render); return;
        }
        BITMAPINFO info{};
        info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        info.bmiHeader.biWidth = surfaceWidth;
        info.bmiHeader.biHeight = -surfaceHeight;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        // One complete presentation: no window-DC erase, image, or individual marker draws.
        if (StretchDIBits(dc, 0, 0, surfaceWidth, surfaceHeight, 0, 0, surfaceWidth, surfaceHeight,
            surface.data(), &info, DIB_RGB_COLORS, SRCCOPY) != surfaceHeight || !GdiFlush()) {
            fail(Error::Render); return;
        }
        // Flush exposure restores too, before a later requested render reuses the CPU surface.
        painted = true;
    }

    static LRESULT CALLBACK window_proc(HWND handle, UINT message, WPARAM wparam, LPARAM lparam) {
        auto* owner = reinterpret_cast<Fixture*>(GetWindowLongPtrW(handle, GWLP_USERDATA));
        if (message == WM_NCCREATE) {
            owner = static_cast<Fixture*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
            SetLastError(ERROR_SUCCESS);
            if (!SetWindowLongPtrW(handle, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(owner)) &&
                GetLastError() != ERROR_SUCCESS) return FALSE;
        }
        if (message == WM_MOUSEACTIVATE) return MA_NOACTIVATE;
        if (message == WM_NCHITTEST) return HTTRANSPARENT;
        if (owner) {
            if ((message == WM_ACTIVATE && LOWORD(wparam) != WA_INACTIVE) || message == WM_SETFOCUS)
                owner->fail(Error::Foreground);
            if (message == WM_DPICHANGED) { owner->fail(Error::Geometry); return 0; }
            if (message == WM_CLOSE) { owner->fail(Error::Closed); return 0; }
            if (message == WM_ERASEBKGND) return 1;
            if (message == WM_PAINT) {
                PAINTSTRUCT paint{};
                HDC dc = BeginPaint(handle, &paint);
                if (!dc) owner->fail(Error::Render);
                else if (owner->counter) owner->draw(dc);
                if (!EndPaint(handle, &paint)) owner->fail(Error::Render);
                return 0;
            }
            // Closing this window must not post WM_QUIT: command polling stays alive.
            if (message == WM_NCDESTROY) SetWindowLongPtrW(handle, GWLP_USERDATA, 0);
        }
        return DefWindowProcW(handle, message, wparam, lparam);
    }

    bool render(bool first = false) {
        if (!window) { fail(Error::Closed); return false; }
        if (!foreground_unchanged() || !geometry_valid()) return false;
        if (counter == std::numeric_limits<uint32_t>::max()) { fail(Error::Render); return false; }
        uint64_t start = 0, end = 0;
        if (!clock.now(start)) { fail(Error::Clock); return false; }
        ++counter;
        if (!compose()) return false;
        painted = false;
        if (first) ShowWindow(window, SW_SHOWNOACTIVATE);
        if (!RedrawWindow(window, nullptr, nullptr, RDW_INVALIDATE | RDW_UPDATENOW) ||
            !painted) fail(Error::Render);
        // Count requested renders only. Exposure restores the same token, without a new event.
        if (!clock.now(end) || !statistics.record(start, end)) fail(Error::Clock);
        return error == Error::None && foreground_unchanged() && geometry_valid();
    }

    bool acknowledge() {
        if (error != Error::None || !foreground_unchanged() || (window && !geometry_valid())) return false;
        char row[512];
        const int length = std::snprintf(row, sizeof(row), "ACK %s %u %lu %llu %d %d %u %u\n",
            nonce.data(), sequence, GetCurrentProcessId(), static_cast<unsigned long long>(windowId),
            width, height, counter, state);
        if (length <= 0 || static_cast<size_t>(length) >= sizeof(row) ||
            !control.atomic_write(Ack, AckTemp, row, static_cast<size_t>(length))) fail(Error::Output);
        return error == Error::None;
    }

    bool write_statistics() {
        uint64_t now = 0, cpu = 0;
        if (!clock.now(now) || now < statistics.wallStart) { fail(Error::Clock); return false; }
        const bool cpuAvailable = statistics.cpuAvailable && cpu_time(cpu) && cpu >= statistics.cpuStart;
        PROCESS_MEMORY_COUNTERS memory{};
        memory.cb = sizeof(memory);
        const bool rssAvailable = GetProcessMemoryInfo(GetCurrentProcess(), &memory, sizeof(memory)) != FALSE;
        Json json;
        json.append("{\"schema\":1,\"nonce\":\"%s\",\"pid\":%lu,\"wall_ns\":%llu,\"cpu_ns\":",
            nonce.data(), GetCurrentProcessId(), static_cast<unsigned long long>(now - statistics.wallStart));
        json.number(cpuAvailable, cpuAvailable ? cpu - statistics.cpuStart : 0);
        json.append(",\"peak_rss_bytes\":");
        json.number(rssAvailable, static_cast<uint64_t>(memory.PeakWorkingSetSize));
        json.append(",\"paint_counter\":%u,\"render_intervals_ns\":[", counter);
        for (size_t index = 0; index < statistics.intervalCount; ++index)
            json.append("%s%llu", index ? "," : "", static_cast<unsigned long long>(statistics.intervals[index]));
        json.append("],\"draw_durations_ns\":[");
        for (size_t index = 0; index < statistics.durationCount; ++index)
            json.append("%s%llu", index ? "," : "", static_cast<unsigned long long>(statistics.durations[index]));
        json.append("],\"lost_samples\":%llu}\n", static_cast<unsigned long long>(statistics.lost));
        if (!json.valid || !control.atomic_write(Metrics, MetricsTemp, json.bytes.data(), json.length)) fail(Error::Output);
        return error == Error::None;
    }

    bool read_command(Operation& operation, uint32_t& next, bool& present) {
        present = false;
        Handle file(CreateFileW(control.paths[Command].c_str(), GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, nullptr));
        if (!file.valid()) {
            if (GetLastError() != ERROR_FILE_NOT_FOUND) fail(Error::File);
            return error == Error::None;
        }
        uint64_t size = 0;
        char row[129];
        DWORD count = 0;
        if (!control.regular(file.value, true, 128, &size) || !size ||
            !ReadFile(file.value, row, sizeof(row), &count, nullptr) || count != size) {
            fail(Error::File); return false;
        }
        if (count < 21 || row[count - 1] != '\n' || std::memcmp(row, nonce.data(), 16) || row[16] != ' ') {
            fail(Error::Command); return false;
        }
        for (DWORD index = 0; index + 1 < count; ++index) {
            if (row[index] < ' ' || row[index] > '~') { fail(Error::Command); return false; }
        }
        size_t offset = 17;
        next = 0;
        if (row[offset] < '1' || row[offset] > '9') { fail(Error::Sequence); return false; }
        while (offset < count && row[offset] >= '0' && row[offset] <= '9') {
            next = next * 10 + static_cast<unsigned>(row[offset++] - '0');
            if (next > 128) { fail(Error::Sequence); return false; }
        }
        if (offset >= count || row[offset++] != ' ') { fail(Error::Command); return false; }
        row[count - 1] = '\0';
        bool known = false;
        for (size_t index = 0; index < sizeof(Operations) / sizeof(Operations[0]); ++index) {
            if (!std::strcmp(row + offset, Operations[index])) {
                operation = static_cast<Operation>(index);
                known = true;
                break;
            }
        }
        if (!known) { fail(Error::Command); return false; }
        if (next == sequence) {
            if (operation != lastOperation) fail(Error::Sequence);
            return error == Error::None;
        }
        if (next != sequence + 1) { fail(Error::Sequence); return false; }
        present = true;
        return true;
    }

    bool close_window() {
        animating = false;
        burstRemaining = 0;
        if (!window) return true;
        HWND previous = window;
        if (!DestroyWindow(previous) || IsWindow(previous)) { fail(Error::Cleanup); return false; }
        window = nullptr;
        return true;
    }

    bool execute(Operation operation, uint32_t next) {
        sequence = next;
        lastOperation = operation;
        bool repaint = false;
        switch (operation) {
        case Operation::Blank: state = 0; repaint = true; break;
        case Operation::Show: state = 1; repaint = true; break;
        case Operation::Pulse: repaint = true; break;
        case Operation::Animate:
        case Operation::Burst:
            state = 1;
            animating = false;
            burstRemaining = 0;
            repaint = true;
            break;
        case Operation::Pause: animating = false; burstRemaining = 0; break;
        case Operation::Resize:
            if (!window) { fail(Error::Closed); return false; }
            animating = false;
            burstRemaining = 0;
            state = 1;
            width = 1040;
            height = 640;
            if (!SetWindowPos(window, nullptr, 0, 0, width, height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW)) { fail(Error::Geometry); return false; }
            repaint = true;
            break;
        case Operation::ResetStats:
            if (!statistics.reset(clock)) { fail(Error::Clock); return false; }
            break;
        case Operation::Stats: if (!write_statistics()) return false; break;
        case Operation::Close: if (!close_window()) return false; break;
        case Operation::Quit:
            animating = false;
            burstRemaining = 0;
            quitting = true;
            break;
        }
        if ((repaint && !render()) || !acknowledge()) return false;
        if (operation == Operation::Animate || operation == Operation::Burst) {
            uint64_t now = 0;
            if (!clock.now(now) || now > std::numeric_limits<uint64_t>::max() - AnimationNanoseconds) {
                fail(Error::Clock); return false;
            }
            // Start after ACK, including all eight burst frames.
            nextRender = now + AnimationNanoseconds;
            burstRemaining = operation == Operation::Burst ? 8 : 0;
            animating = true;
        }
        return true;
    }

    void run(int argc, wchar_t** argv) {
        if (argc != 4) { fail(Error::Arguments); return; }
        if (!control.open(argv[1])) { fail(Error::Root); return; }
        if (!fuse.start(control)) { fail(Error::Native); return; }
        if (control.stop_requested(error) || error != Error::None) return;
        if (!control.outputs_absent()) { fail(Error::File); return; }
        if (wcslen(argv[2]) != 16) { fail(Error::Arguments); return; }
        for (size_t index = 0; index < 16; ++index) {
            const wchar_t character = argv[2][index];
            const int digit = character >= L'0' && character <= L'9' ? character - L'0' :
                character >= L'a' && character <= L'f' ? character - L'a' + 10 : -1;
            if (digit < 0) { fail(Error::Arguments); return; }
            nonce[index] = static_cast<char>(character);
            nonceBits = (nonceBits << 4) | static_cast<unsigned>(digit);
        }
        foreground = GetForegroundWindow();
        if (!foreground) { fail(Error::Foreground); return; }
        foregroundObserverOwner = this;
        foregroundHook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, nullptr,
            foreground_event, 0, 0, WINEVENT_OUTOFCONTEXT);
        if (!foregroundHook) { fail(Error::Native); return; }
        if (!clock.initialize() || !statistics.reset(clock)) { fail(Error::Clock); return; }
        Handle source(CreateFileW(argv[3], GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, nullptr));
        uint64_t size = 0;
        DWORD count = 0;
        if (!source.valid() || !control.regular(source.value, false, ImageBytes, &size) || size != ImageBytes) {
            fail(Error::Image); return;
        }
        image.resize(ImageBytes);
        if (!ReadFile(source.value, image.data(), static_cast<DWORD>(ImageBytes), &count, nullptr) ||
            count != ImageBytes || !source.close()) { fail(Error::Image); return; }
        surface.resize(SurfacePixels);
        if (!SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)) { fail(Error::Native); return; }
        MONITORINFO monitor{sizeof(MONITORINFO)};
        const POINT origin{0, 0};
        if (!GetMonitorInfoW(MonitorFromPoint(origin, MONITOR_DEFAULTTOPRIMARY), &monitor) ||
            monitor.rcWork.right - monitor.rcWork.left < 1088 || monitor.rcWork.bottom - monitor.rcWork.top < 704) {
            fail(Error::Geometry); return;
        }
        instance = GetModuleHandleW(nullptr);
        WNDCLASSW type{};
        type.lpfnWndProc = window_proc;
        type.hInstance = instance;
        type.lpszClassName = ClassName;
        if (!instance || !RegisterClassW(&type)) { fail(Error::Native); return; }
        classRegistered = true;
        const std::wstring title = std::wstring(L"MadoPilot Pacing ") + argv[2];
        window = CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT, ClassName,
            title.c_str(), WS_POPUP, monitor.rcWork.left + 32, monitor.rcWork.top + 32,
            width, height, nullptr, nullptr, instance, this);
        if (!window) { fail(Error::Native); return; }
        windowId = static_cast<uint64_t>(reinterpret_cast<ULONG_PTR>(window));
        const DWM_WINDOW_CORNER_PREFERENCE corners = DWMWCP_DONOTROUND;
        if (FAILED(DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &corners, sizeof(corners)))) {
            fail(Error::Native); return;
        }
        if (!render(true) || !acknowledge()) return;
        while (error == Error::None && !quitting) {
            if (control.stop_requested(error)) break;
            if (error != Error::None || !foreground_unchanged()) break;
            Operation operation = Operation::Blank;
            uint32_t next = 0;
            bool present = false;
            if (!read_command(operation, next, present) || (present && !execute(operation, next))) break;
            if (quitting) break;
            MSG message{};
            // A bounded pump prevents queued GUI messages from starving stop/command files.
            for (unsigned pumped = 0; pumped < 64 && PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE); ++pumped) {
                if (message.message == WM_QUIT) { fail(Error::Native); break; }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if (error != Error::None || !foreground_unchanged()) break;
            uint64_t now = 0;
            if (!clock.now(now)) { fail(Error::Clock); break; }
            if (animating && now >= nextRender) {
                if (!render()) break;
                if (burstRemaining && --burstRemaining == 0) animating = false;
                if (!clock.now(now) || now > std::numeric_limits<uint64_t>::max() - AnimationNanoseconds) {
                    fail(Error::Clock); break;
                }
                nextRender += AnimationNanoseconds;
                if (nextRender <= now) nextRender = now + AnimationNanoseconds; // No catch-up burst.
            }
            DWORD wait = 8;
            if (animating && nextRender > now) {
                const uint64_t remaining = (nextRender - now + 999999) / 1000000;
                if (remaining < wait) wait = static_cast<DWORD>(remaining);
            }
            if (MsgWaitForMultipleObjectsEx(0, nullptr, wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE) == WAIT_FAILED) {
                fail(Error::Native); break;
            }
        }
    }

    void cleanup() {
        close_window();
        if (classRegistered && !UnregisterClassW(ClassName, instance)) fail(Error::Cleanup);
        classRegistered = false;
        if (foreground && !foreground_unchanged()) fail(Error::Foreground);
        if (foregroundHook && !UnhookWinEvent(foregroundHook)) fail(Error::Cleanup);
        foregroundHook = nullptr;
        foregroundObserverOwner = nullptr;
        if (!fuse.stop()) fail(Error::Cleanup);
    }
};
} // namespace

int wmain(int argc, wchar_t** argv) {
    Fixture fixture;
    try { fixture.run(argc, argv); }
    catch (...) { fixture.fail(Error::Exception); }
    fixture.cleanup();
    if (fixture.error != Error::None) {
        fixture.control.report(fixture.error);
        return fixture.error == Error::Arguments || fixture.error == Error::Root ? 2 : 1;
    }
    return 0;
}
