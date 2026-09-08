/* Private native qualification transport shared by independent C and C++
 * consumers. No product header, capture implementation, or token codec lives
 * here. The controller supplies the existing codec's complete 10x9 cell grid.
 */
#ifndef MADOPILOT_NATIVE_WATCH_PROTOCOL_H
#define MADOPILOT_NATIVE_WATCH_PROTOCOL_H

#include <errno.h>
#include <inttypes.h>
#include <limits.h>
#include <math.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <psapi.h>
#if defined(_MSC_VER)
#pragma comment(lib, "psapi.lib")
#endif
#else
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <time.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <dlfcn.h>
#include <mach/mach.h>
#include <mach/task_info.h>
#endif
#endif

#define MPW_LINE_MAX 2048u
#define MPW_REPLY_MILLIS UINT64_C(5000)
#define MPW_MAX_VISUAL_COMMANDS 64u

typedef struct mpw_shape {
    uint32_t marker_cell_w, marker_cell_h;
    int32_t marker_x, marker_y;
    uint32_t token_cell_w, token_cell_h;
    int32_t token_x, token_y;
} mpw_shape;

typedef struct mpw_token {
    uint32_t value;
    int visible;
    char cells[91];
} mpw_token;

typedef struct mpw_resource_snapshot {
    uint64_t private_or_footprint_bytes;
    uint64_t resident_bytes;
    uint64_t handle_or_port_count;
} mpw_resource_snapshot;

/* Same-process OS observations, not GPU bytes, Rust heap, or exact live native
 * object counts. A failed read has no sample. The observer's Mach out-of-line
 * arrays are deallocated on every path and their entries are never inspected. */
static inline int mpw_resources(mpw_resource_snapshot *out)
{
    if (out == NULL) return 0;
    memset(out, 0, sizeof(*out));
#if defined(_WIN32)
    {
        PROCESS_MEMORY_COUNTERS_EX memory;
        DWORD handles = 0;
        memset(&memory, 0, sizeof(memory));
        memory.cb = (DWORD)sizeof(memory);
        if (!GetProcessMemoryInfo(GetCurrentProcess(), (PROCESS_MEMORY_COUNTERS *)&memory, (DWORD)sizeof(memory)) ||
            !GetProcessHandleCount(GetCurrentProcess(), &handles)) return 0;
        out->private_or_footprint_bytes = (uint64_t)memory.PrivateUsage;
        out->resident_bytes = (uint64_t)memory.WorkingSetSize;
        out->handle_or_port_count = handles;
    }
#elif defined(__APPLE__)
    {
        task_vm_info_data_t memory;
        mach_msg_type_number_t memory_count = TASK_VM_INFO_COUNT;
        mach_port_name_array_t names = NULL;
        mach_port_type_array_t types = NULL;
        mach_msg_type_number_t name_count = 0, type_count = 0;
        kern_return_t status;
        int released_arrays = 1;
        memset(&memory, 0, sizeof(memory));
        if (task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&memory, &memory_count) != KERN_SUCCESS ||
            memory_count < TASK_VM_INFO_REV1_COUNT) return 0;
        status = mach_port_names(mach_task_self(), &names, &name_count, &types, &type_count);
        if (names != NULL && vm_deallocate(mach_task_self(), (vm_address_t)(uintptr_t)names,
                                          (vm_size_t)name_count * sizeof(*names)) != KERN_SUCCESS) released_arrays = 0;
        if (types != NULL && vm_deallocate(mach_task_self(), (vm_address_t)(uintptr_t)types,
                                          (vm_size_t)type_count * sizeof(*types)) != KERN_SUCCESS) released_arrays = 0;
        if (status != KERN_SUCCESS || !released_arrays || name_count != type_count) return 0;
        out->private_or_footprint_bytes = (uint64_t)memory.phys_footprint;
        out->resident_bytes = (uint64_t)memory.resident_size;
        out->handle_or_port_count = name_count;
    }
#else
    return 0;
#endif
    return 1;
}

/* These helpers are all static inline: either consumer may use only a subset
 * without -Wunused-function warnings. Each executable has one protocol thread.
 */
static inline uint64_t mpw_now_millis(void)
{
#if defined(_WIN32)
    return (uint64_t)GetTickCount64();
#else
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now) != 0 || now.tv_sec < 0) return UINT64_MAX;
    if ((uint64_t)now.tv_sec > (UINT64_MAX - 999u) / 1000u) return UINT64_MAX;
    return (uint64_t)now.tv_sec * 1000u + (uint64_t)now.tv_nsec / 1000000u;
#endif
}

static inline void mpw_pause(void)
{
#if defined(_WIN32)
    Sleep(5u);
#else
    const struct timespec delay = { 0, 5000000 };
    (void)nanosleep(&delay, NULL);
#endif
}

static inline int mpw_finite(double value)
{
    /* Portable to both C and C++, including the MSVC C library. */
    return value == value && value <= 1.0e100 && value >= -1.0e100;
}

static inline int mpw_identifier(const char *text, size_t maximum)
{
    size_t index;
    if (text == NULL || text[0] == '\0') return 0;
    for (index = 0; index <= maximum; ++index) {
        unsigned char ch = (unsigned char)text[index];
        if (ch == 0) return 1;
        if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
              (ch >= '0' && ch <= '9') || ch == '_')) return 0;
    }
    return 0;
}

static inline int mpw_valid_row(const char *row)
{
    return row != NULL && row[0] == 'F' && row[1] >= '1' && row[1] <= '9' && row[2] == '\0';
}

/* One request is outstanding at a time. A reply is read only after the complete
 * request is flushed. On Windows the reported outgoing pipe quota must hold
 * the sole <=2048-byte request; the previous reply proves the previous request
 * was drained. No worker thread or unbounded reader queue is created.
 * Transport failure poisons this conversation: later cleanup cannot consume a
 * late reply as the acknowledgement of a different request.
 */
static inline int mpw_exchange(const char *request, char *reply, size_t capacity)
{
    static int poisoned = 0;
    char output[MPW_LINE_MAX];
    uint64_t start, deadline;
    size_t size = 0, used = 0, index;
    int success = 0, carriage_return = 0;
#if !defined(_WIN32)
    int old_flags = -1;
#endif
    if (reply != NULL && capacity != 0) reply[0] = '\0';
    if (poisoned || request == NULL || reply == NULL || capacity < 2) return 0;
    while (size < MPW_LINE_MAX - 1 && request[size] != '\0') ++size;
    if (size == 0 || (size == MPW_LINE_MAX - 1 && request[size] != '\0')) return 0;
    for (index = 0; index < size; ++index) {
        if ((unsigned char)request[index] < 32 || (unsigned char)request[index] == 127) return 0;
    }
    memcpy(output, request, size);
    output[size++] = '\n';
    start = mpw_now_millis();
    if (start > UINT64_MAX - MPW_REPLY_MILLIS) return 0;
    deadline = start + MPW_REPLY_MILLIS;
    poisoned = 1;
#if defined(_WIN32)
    {
        HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
        HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
        DWORD written = 0, outgoing = 0;
        if (GetFileType(out) != FILE_TYPE_PIPE || GetFileType(in) != FILE_TYPE_PIPE ||
            !GetNamedPipeInfo(out, NULL, &outgoing, NULL, NULL) || outgoing < (DWORD)size ||
            !WriteFile(out, output, (DWORD)size, &written, NULL) || written != (DWORD)size) return 0;
        while (mpw_now_millis() < deadline) {
            DWORD available = 0, count = 0;
            unsigned char ch;
            if (!PeekNamedPipe(in, NULL, 0, NULL, &available, NULL)) break;
            if (available == 0) { mpw_pause(); continue; }
            if (!ReadFile(in, &ch, 1, &count, NULL) || count != 1) break;
            if (ch == '\n') { success = used != 0; break; }
            if (carriage_return) break;
            if (ch == '\r') {
                if (used >= MPW_LINE_MAX - 1) break;
                carriage_return = 1; continue;
            }
            if (ch < 32 || ch == 127 || used >= MPW_LINE_MAX - 1 || used + 1 >= capacity) break;
            reply[used++] = (char)ch;
        }
    }
#else
    {
        struct pollfd ready;
        ssize_t written;
        (void)signal(SIGPIPE, SIG_IGN);
        old_flags = fcntl(STDOUT_FILENO, F_GETFL);
        if (old_flags < 0 || fcntl(STDOUT_FILENO, F_SETFL, old_flags | O_NONBLOCK) < 0) return 0;
        written = write(STDOUT_FILENO, output, size);
        if (fcntl(STDOUT_FILENO, F_SETFL, old_flags) < 0 || written != (ssize_t)size) return 0;
        ready.fd = STDIN_FILENO;
        ready.events = POLLIN;
        while (mpw_now_millis() < deadline) {
            uint64_t now = mpw_now_millis();
            unsigned char ch;
            ssize_t count;
            int state;
            if (now >= deadline) break;
            ready.revents = 0;
            state = poll(&ready, 1, (int)(deadline - now));
            if (state < 0 && errno == EINTR) continue;
            if (state <= 0 || (ready.revents & POLLIN) == 0) break;
            count = read(STDIN_FILENO, &ch, 1);
            if (count < 0 && errno == EINTR) continue;
            if (count != 1) break;
            if (ch == '\n') { success = used != 0; break; }
            if (carriage_return) break;
            if (ch == '\r') {
                if (used >= MPW_LINE_MAX - 1) break;
                carriage_return = 1; continue;
            }
            if (ch < 32 || ch == 127 || used >= MPW_LINE_MAX - 1 || used + 1 >= capacity) break;
            reply[used++] = (char)ch;
        }
    }
#endif
    if (!success || mpw_now_millis() >= deadline) { reply[0] = '\0'; return 0; }
    reply[used] = '\0';
    poisoned = 0;
    return 1;
}

/* Decimal-only parser: no signs, whitespace, overflow, or alternate radix. */
static inline int mpw_unsigned(const char **cursor, uint64_t ceiling, uint64_t *value)
{
    const char *next = *cursor;
    uint64_t result = 0;
    if (*next < '0' || *next > '9') return 0;
    do {
        uint64_t digit = (uint64_t)(*next - '0');
        if (digit > ceiling || result > (ceiling - digit) / 10u) return 0;
        result = result * 10u + digit;
        ++next;
    } while (*next >= '0' && *next <= '9');
    *cursor = next;
    *value = result;
    return 1;
}

static inline int mpw_space(const char **cursor)
{
    if (**cursor != ' ') return 0;
    ++*cursor;
    return **cursor != ' ' && **cursor != '\0';
}

static inline int mpw_shape_fits(uint32_t width, uint32_t height, const mpw_shape *shape)
{
    if (shape == NULL || width == 0 || height == 0 || width > INT32_MAX || height > INT32_MAX ||
        shape->marker_cell_w == 0 || shape->marker_cell_h == 0 ||
        shape->token_cell_w == 0 || shape->token_cell_h == 0 ||
        shape->marker_x < 0 || shape->marker_y < 0 || shape->token_x < 0 || shape->token_y < 0) return 0;
    return (uint64_t)(uint32_t)shape->marker_x + (uint64_t)shape->marker_cell_w * 3u <= width &&
           (uint64_t)(uint32_t)shape->marker_y + (uint64_t)shape->marker_cell_h * 2u <= height &&
           (uint64_t)(uint32_t)shape->token_x + (uint64_t)shape->token_cell_w * 10u <= width &&
           (uint64_t)(uint32_t)shape->token_y + (uint64_t)shape->token_cell_h * 9u <= height;
}

static inline int mpw_visual(int visible, mpw_token *out)
{
    static uint32_t issued[MPW_MAX_VISUAL_COMMANDS];
    static size_t count = 0;
    char reply[MPW_LINE_MAX + 1];
    const char *cursor;
    uint64_t token_value, marker;
    size_t index;
    if (out == NULL) return 0;
    memset(out, 0, sizeof(*out));
    if ((visible != 0 && visible != 1) || count == MPW_MAX_VISUAL_COMMANDS ||
        !mpw_exchange(visible ? "VISUAL visible" : "VISUAL absent", reply, sizeof(reply)) ||
        strncmp(reply, "ACK ", 4) != 0) return 0;
    cursor = reply + 4;
    if (!mpw_unsigned(&cursor, UINT32_MAX, &token_value) || token_value == 0 ||
        !mpw_space(&cursor) || !mpw_unsigned(&cursor, 1, &marker) || marker != (uint64_t)visible ||
        !mpw_space(&cursor) || strlen(cursor) != 90 || cursor[0] != '1' || cursor[1] != '0') return 0;
    for (index = 0; index < 90; ++index) if (cursor[index] != '0' && cursor[index] != '1') return 0;
    for (index = 0; index < count; ++index) if (issued[index] == (uint32_t)token_value) return 0;
    issued[count++] = (uint32_t)token_value;
    out->value = (uint32_t)token_value;
    out->visible = visible;
    memcpy(out->cells, cursor, 91);
    return 1;
}

static inline int mpw_geometry(uint32_t width, uint32_t height,
                                double desktop_x, double desktop_y,
                                double logical_w, double logical_h,
                                double scale_x, double scale_y, mpw_shape *out)
{
    char request[MPW_LINE_MAX], reply[MPW_LINE_MAX + 1];
    const char *cursor;
    uint64_t values[8];
    size_t index;
    int length;
    mpw_shape shape;
    if (out == NULL) return 0;
    memset(out, 0, sizeof(*out));
    if (width == 0 || height == 0 || width > INT32_MAX || height > INT32_MAX ||
        !mpw_finite(desktop_x) || !mpw_finite(desktop_y) || !mpw_finite(logical_w) ||
        !mpw_finite(logical_h) || !mpw_finite(scale_x) || !mpw_finite(scale_y) ||
        logical_w <= 0 || logical_h <= 0 || scale_x <= 0 || scale_y <= 0) return 0;
    length = snprintf(request, sizeof(request), "GEOMETRY %" PRIu32 " %" PRIu32
                      " %.17g %.17g %.17g %.17g %.17g %.17g", width, height,
                      desktop_x, desktop_y, logical_w, logical_h, scale_x, scale_y);
    if (length < 0 || (size_t)length >= sizeof(request) ||
        !mpw_exchange(request, reply, sizeof(reply)) || strncmp(reply, "SHAPE ", 6) != 0) return 0;
    cursor = reply + 6;
    for (index = 0; index < 8; ++index) {
        uint64_t ceiling = (index == 2 || index == 3 || index == 6 || index == 7) ? INT32_MAX : UINT32_MAX;
        if (!mpw_unsigned(&cursor, ceiling, &values[index]) ||
            (index != 7 && !mpw_space(&cursor))) return 0;
    }
    if (*cursor != '\0') return 0;
    shape.marker_cell_w = (uint32_t)values[0]; shape.marker_cell_h = (uint32_t)values[1];
    shape.marker_x = (int32_t)values[2]; shape.marker_y = (int32_t)values[3];
    shape.token_cell_w = (uint32_t)values[4]; shape.token_cell_h = (uint32_t)values[5];
    shape.token_x = (int32_t)values[6]; shape.token_y = (int32_t)values[7];
    if (!mpw_shape_fits(width, height, &shape)) return 0;
    *out = shape;
    return 1;
}

/* Reject malformed UTF-8, overlong encodings, surrogates and controls. Paths
 * stay transient: the validated package loader, not this transport, owns
 * filesystem and package security policy. */
static inline int mpw_utf8_payload(const char *text)
{
    const unsigned char *cursor = (const unsigned char *)text;
    if (cursor == NULL || *cursor == 0) return 0;
    while (*cursor != 0) {
        uint32_t scalar, minimum;
        unsigned remaining, index;
        unsigned char first = *cursor++;
        if (first < 0x80) {
            if (first < 32 || first == 127) return 0;
            continue;
        }
        if (first >= 0xc2 && first <= 0xdf) { scalar = first & 0x1fu; remaining = 1; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { scalar = first & 0x0fu; remaining = 2; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { scalar = first & 7u; remaining = 3; minimum = 0x10000; }
        else return 0;
        for (index = 0; index < remaining; ++index) {
            if (*cursor < 0x80 || *cursor > 0xbf) return 0;
            scalar = (scalar << 6) | (*cursor++ & 0x3fu);
        }
        if (scalar < minimum || scalar > 0x10ffff || (scalar >= 0xd800 && scalar <= 0xdfff)) return 0;
    }
    return 1;
}

static inline int mpw_library(const void *entry_address)
{
    char request[MPW_LINE_MAX], reply[MPW_LINE_MAX + 1];
    const char *path;
    int written;
#if defined(_WIN32)
    HMODULE module = NULL;
    wchar_t wide[MPW_LINE_MAX];
    char utf8[MPW_LINE_MAX];
    DWORD length;
    if (entry_address == NULL ||
        !GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                           GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                           (LPCWSTR)entry_address, &module)) return 0;
    length = GetModuleFileNameW(module, wide, MPW_LINE_MAX);
    if (length == 0 || length >= MPW_LINE_MAX ||
        WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide, -1, utf8,
                            (int)sizeof(utf8), NULL, NULL) == 0) return 0;
    path = utf8;
#elif defined(__APPLE__)
    Dl_info info;
    memset(&info, 0, sizeof(info));
    if (entry_address == NULL || dladdr(entry_address, &info) == 0 || info.dli_fname == NULL) return 0;
    path = info.dli_fname;
#else
    (void)entry_address;
    return 0;
#endif
#if defined(_WIN32) || defined(__APPLE__)
    if (!mpw_utf8_payload(path)) return 0;
    written = snprintf(request, sizeof(request), "LOADED %s", path);
    return written > 0 && (size_t)written < sizeof(request) &&
           mpw_exchange(request, reply, sizeof(reply)) && strcmp(reply, "OK") == 0;
#else
    (void)request; (void)reply; (void)path; (void)written;
#endif
}

static inline int mpw_asset(uint32_t cell_w, uint32_t cell_h, char *path, size_t capacity)
{
    char request[96], reply[MPW_LINE_MAX + 1];
    size_t length;
    int written;
    if (path == NULL || capacity == 0) return 0;
    path[0] = '\0';
    if (cell_w == 0 || cell_h == 0 || cell_w > (uint32_t)INT32_MAX / 3u ||
        cell_h > (uint32_t)INT32_MAX / 2u || (uint64_t)cell_w * cell_h > UINT64_C(1048576)) return 0;
    written = snprintf(request, sizeof(request), "ASSET %" PRIu32 " %" PRIu32, cell_w, cell_h);
    if (written < 0 || (size_t)written >= sizeof(request) ||
        !mpw_exchange(request, reply, sizeof(reply)) || strncmp(reply, "PACKAGE ", 8) != 0) return 0;
    length = strlen(reply + 8);
    if (length == 0 || length >= capacity || !mpw_utf8_payload(reply + 8)) return 0;
    memcpy(path, reply + 8, length + 1);
    return 1;
}

static inline int mpw_action(const char *action)
{
    char reply[MPW_LINE_MAX + 1];
    int optional;
    if (action == NULL) return 0;
    optional = strcmp(action, "RESIZE") == 0 || strcmp(action, "MOVE") == 0;
    if (!optional && strcmp(action, "DESTROY") != 0 && strcmp(action, "DONE") != 0) return 0;
    if (!mpw_exchange(action, reply, sizeof(reply))) return 0;
    if (strcmp(reply, "OK") == 0) return 1;
    return optional && strcmp(reply, "UNSUPPORTED") == 0 ? -1 : 0;
}

static inline int mpw_row(const char *row, const char *outcome, const char *reason)
{
    char request[160], reply[MPW_LINE_MAX + 1];
    int written;
    if (!mpw_valid_row(row) || outcome == NULL || !mpw_identifier(reason, 64) ||
        (strcmp(outcome, "PASS") != 0 && strcmp(outcome, "FAIL") != 0 &&
         strcmp(outcome, "UNSUPPORTED") != 0 && strcmp(outcome, "UNEXECUTED") != 0 &&
         strcmp(outcome, "INFRA") != 0)) return 0;
    written = snprintf(request, sizeof(request), "ROW %s %s %s", row, outcome, reason);
    return written > 0 && (size_t)written < sizeof(request) &&
           mpw_exchange(request, reply, sizeof(reply)) && strcmp(reply, "OK") == 0;
}

static inline int mpw_fact(const char *row, const char *key, const char *numeric_values)
{
    char request[512], reply[MPW_LINE_MAX + 1];
    const char *cursor = numeric_values;
    size_t arity = 0, index;
    int written;
    if (!mpw_valid_row(row) || key == NULL || cursor == NULL) return 0;
    if (strcmp(key, "frame") == 0 || strcmp(key, "bootstrap") == 0) arity = 6;
    else if (strcmp(key, "transform") == 0) arity = 8;
    else if (strcmp(key, "match") == 0) arity = 9;
    else if (strcmp(key, "status") == 0 || strcmp(key, "permissions") == 0) arity = 2;
    else if (strcmp(key, "pending") == 0) arity = 3;
    else if (strcmp(key, "resource") == 0) arity = 5;
    else if (strcmp(key, "startup_nanos") == 0 || strcmp(key, "elapsed_nanos") == 0 ||
             strcmp(key, "mapped_view_bytes") == 0) arity = 1;
    else return 0;
    for (index = 0; index < arity; ++index) {
        if (strcmp(key, "transform") == 0 && index >= 1 && index <= 6) {
            char *end;
            const char *begin = cursor;
            double value;
            errno = 0;
            value = strtod(cursor, &end);
            if (end == cursor || errno == ERANGE || !mpw_finite(value) || end - cursor > 32 ||
                (index >= 3 && value <= 0)) return 0;
            while (begin != end) {
                char ch = *begin++;
                if (!((ch >= '0' && ch <= '9') || ch == '-' || ch == '+' || ch == '.' || ch == 'e' || ch == 'E')) return 0;
            }
            cursor = end;
        } else {
            uint64_t value, ceiling = UINT64_MAX;
            int negative = 0;
            if (strcmp(key, "match") == 0 && index >= 5) {
                negative = *cursor == '-';
                if (negative) ++cursor;
                ceiling = negative ? (uint64_t)INT32_MAX + 1u : INT32_MAX;
            } else if (((strcmp(key, "frame") == 0 || strcmp(key, "bootstrap") == 0) && index >= 4) ||
                       (strcmp(key, "match") == 0 && index == 3) ||
                       (strcmp(key, "pending") == 0 && index == 1)) ceiling = UINT32_MAX;
            else if (strcmp(key, "permissions") == 0) ceiling = 3;
            else if (strcmp(key, "status") == 0) ceiling = index == 0 ? 13 : 8;
            else if (strcmp(key, "resource") == 0 && index < 2) ceiling = index == 0 ? 2 : 4;
            if (!mpw_unsigned(&cursor, ceiling, &value)) return 0;
        }
        if (index + 1 < arity && !mpw_space(&cursor)) return 0;
    }
    if (*cursor != '\0') return 0;
    written = snprintf(request, sizeof(request), "FACT %s %s %s", row, key, numeric_values);
    return written > 0 && (size_t)written < sizeof(request) &&
           mpw_exchange(request, reply, sizeof(reply)) && strcmp(reply, "OK") == 0;
}

/* The reference marker_state contract: six center samples, tolerance 8,
 * separation 32, no coordinate search. -1 is malformed or ambiguous, not absent.
 * Alpha does not participate, just as in the Rust native consumer. */
static inline int mpw_marker_state(const uint8_t *pixels, size_t len, uint64_t stride,
                                   uint32_t width, uint32_t height, const mpw_shape *shape)
{
    const uint8_t *cells[6];
    uint64_t row_bytes = (uint64_t)width * 4u, required;
    size_t index, channel;
    int uniform = 1, separated = 0;
    if (pixels == NULL || !mpw_shape_fits(width, height, shape) ||
        stride < row_bytes || stride > SIZE_MAX) return -1;
    if ((uint64_t)(height - 1u) > (UINT64_MAX - row_bytes) / stride) return -1;
    required = (uint64_t)(height - 1u) * stride + row_bytes;
    if (required > SIZE_MAX || required > len) return -1;
    for (index = 0; index < 6; ++index) {
        uint64_t x = (uint32_t)shape->marker_x + (uint64_t)(index % 3u) * shape->marker_cell_w +
                     shape->marker_cell_w / 2u;
        uint64_t y = (uint32_t)shape->marker_y + (uint64_t)(index / 3u) * shape->marker_cell_h +
                     shape->marker_cell_h / 2u;
        cells[index] = pixels + (size_t)(y * stride + x * 4u);
        for (channel = 0; channel < 3; ++channel) {
            if (abs((int)cells[index][channel] - (int)cells[0][channel]) > 8) uniform = 0;
        }
    }
    if (uniform) return 0;
    for (channel = 0; channel < 3; ++channel) {
        if (abs((int)cells[0][channel] - (int)cells[1][channel]) >= 32) separated = 1;
    }
    if (!separated) return -1;
    for (index = 0; index < 6; ++index) {
        const uint8_t *expected = cells[index == 1 || index == 3 ? 1 : 0];
        for (channel = 0; channel < 3; ++channel) {
            if (abs((int)cells[index][channel] - (int)expected[channel]) > 8) return -1;
        }
    }
    return 1;
}

static inline int mpw_pixels_match_token(const uint8_t *pixels, size_t len, uint64_t stride,
                                         uint32_t width, uint32_t height,
                                         const mpw_shape *shape, const mpw_token *token)
{
    const uint8_t *primary, *secondary;
    uint64_t row_bytes = (uint64_t)width * 4u, required;
    size_t index, channel;
    int separated = 0;
    if (pixels == NULL || token == NULL || token->value == 0 ||
        (token->visible != 0 && token->visible != 1) || token->cells[90] != '\0' ||
        token->cells[0] != '1' || token->cells[1] != '0' ||
        !mpw_shape_fits(width, height, shape) || stride < row_bytes || stride > SIZE_MAX) return 0;
    if ((uint64_t)(height - 1u) > (UINT64_MAX - row_bytes) / stride) return 0;
    required = (uint64_t)(height - 1u) * stride + row_bytes;
    if (required > SIZE_MAX || required > len) return 0;
    primary = pixels + (size_t)(((uint64_t)(uint32_t)shape->token_y + shape->token_cell_h / 2u) * stride +
                               ((uint64_t)(uint32_t)shape->token_x + shape->token_cell_w / 2u) * 4u);
    secondary = primary + (size_t)shape->token_cell_w * 4u;
    for (channel = 0; channel < 3; ++channel) {
        if (abs((int)primary[channel] - (int)secondary[channel]) >= 32) separated = 1;
    }
    if (!separated) return 0;
    for (index = 0; index < 90; ++index) {
        uint64_t x = (uint32_t)shape->token_x + (uint64_t)(index % 10u) * shape->token_cell_w + shape->token_cell_w / 2u;
        uint64_t y = (uint32_t)shape->token_y + (uint64_t)(index / 10u) * shape->token_cell_h + shape->token_cell_h / 2u;
        const uint8_t *sample = pixels + (size_t)(y * stride + x * 4u);
        int is_primary = 1, is_secondary = 1;
        if (token->cells[index] != '0' && token->cells[index] != '1') return 0;
        for (channel = 0; channel < 3; ++channel) {
            if (abs((int)sample[channel] - (int)primary[channel]) > 12) is_primary = 0;
            if (abs((int)sample[channel] - (int)secondary[channel]) > 12) is_secondary = 0;
        }
        if (is_primary == is_secondary || is_primary != (token->cells[index] == '1')) return 0;
    }
    return 1;
}

#endif
