/* Private deterministic seams; no library, model, capture, or input execution. */
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <madopilot/madopilot.h>

#ifndef TEST_SAMPLE
#define TEST_SAMPLE 0
#endif

#define CHECK(condition) do { \
    if (!(condition)) { \
        fprintf(stderr, "ingress check failed at line %d: %s\n", __LINE__, #condition); \
        exit(1); \
    } \
} while (0)

static struct { void* pointer; size_t bytes; } allocations[32];
static size_t live;
static size_t allocation_calls;
static size_t fail_allocation;
static size_t conversion_calls;
static size_t fail_conversion;
static int body_calls;
static int body_result;
static int api_calls;
static int borrow_calls;
static int negotiate_ocr;
static int path_calls;
static wchar_t* environment_model;
static wchar_t* environment_runtime;
static const wchar_t* selected_paths[2];
static const char* const* expected_arguments;
static int expected_count;
static wchar_t unicode_path[] = L"C:\\\u65e5\u672c-\xD83D\xDE80";
static const char unicode_utf8[] = "C:\\\xe6\x97\xa5\xe6\x9c\xac-\xf0\x9f\x9a\x80";
static wchar_t malformed[] = {0xd800, 0};
static wchar_t runtime_path[] = L"C:\\\u65e5\u672c-\xD83D\xDE80-runtime";
static wchar_t environment_scratch[128];

static void* test_malloc(size_t bytes)
{
    ++allocation_calls;
    if (allocation_calls == fail_allocation) return NULL;
    void* pointer = malloc(bytes);
    CHECK(pointer != NULL && live < 32u);
    allocations[live].pointer = pointer;
    allocations[live++].bytes = bytes;
    return pointer;
}

static void* test_calloc(size_t count, size_t bytes)
{
    CHECK(count <= SIZE_MAX / bytes);
    void* pointer = test_malloc(count * bytes);
    if (pointer != NULL) memset(pointer, 0, count * bytes);
    return pointer;
}

static void test_free(void* pointer)
{
    if (pointer == NULL) return;
    for (size_t index = 0; index < live; ++index) {
        if (allocations[index].pointer == pointer) {
            memset(pointer, 0xdd, allocations[index].bytes);
            free(pointer);
            allocations[index] = allocations[--live];
            return;
        }
    }
    CHECK(0); /* Foreign or double free. */
}

static int owns_text(const char* text)
{
    for (size_t index = 0; index < live; ++index) {
        if (allocations[index].bytes == strlen(text) + 1u &&
            memcmp(allocations[index].pointer, text, strlen(text) + 1u) == 0) return 1;
    }
    return 0;
}

static int WINAPI test_convert(UINT page, DWORD flags, LPCWCH wide, int length,
                                LPSTR text, int bytes, LPCCH fallback, LPBOOL used)
{
    CHECK(page == CP_UTF8 && flags == WC_ERR_INVALID_CHARS);
    CHECK(length == -1 && fallback == NULL && used == NULL);
    ++conversion_calls;
    if (conversion_calls == fail_conversion) return 0;
    return WideCharToMultiByte(page, flags, wide, length, text, bytes, fallback, used);
}

static wchar_t* test_getenv(const wchar_t* name)
{
    const wchar_t* value;
    if (wcscmp(name, L"MADO_PILOT_G004_MODEL_ROOT") == 0) {
        value = environment_model;
    } else {
        CHECK(wcscmp(name, L"MADO_PILOT_ONNX_RUNTIME") == 0);
        value = environment_runtime;
    }
    /* The next environment read invalidates the previous CRT borrow. */
    memset(environment_scratch, 0xdd, sizeof(environment_scratch));
    if (value == NULL) return NULL;
    CHECK(wcslen(value) < 128u);
    memcpy(environment_scratch, value, (wcslen(value) + 1u) * sizeof(wchar_t));
    return environment_scratch;
}

/* Canonicalization uses a deterministic OS seam, not a real-path acceptance oracle. */
static HANDLE WINAPI test_open(LPCWSTR path, DWORD access, DWORD sharing,
                               LPSECURITY_ATTRIBUTES security, DWORD disposition,
                               DWORD attributes, HANDLE template_file)
{
    (void)access; (void)sharing; (void)security; (void)disposition;
    (void)attributes; (void)template_file;
    CHECK(path_calls < 2);
    ++path_calls;
    if (wcscmp(path, selected_paths[0]) == 0) return (HANDLE)(uintptr_t)1;
    CHECK(wcscmp(path, selected_paths[1]) == 0);
    return (HANDLE)(uintptr_t)2;
}

static DWORD WINAPI test_final_path(HANDLE handle, LPWSTR buffer, DWORD capacity, DWORD flags)
{
    (void)flags;
    size_t index = (size_t)(uintptr_t)handle - 1u;
    CHECK(index < 2u);
    size_t length = wcslen(selected_paths[index]);
    if (capacity == 0) return (DWORD)length + 1u;
    CHECK(capacity > length);
    memcpy(buffer, selected_paths[index], (length + 1u) * sizeof(wchar_t));
    return (DWORD)length;
}

static BOOL WINAPI test_close(HANDLE handle)
{
    CHECK((uintptr_t)handle == 1u || (uintptr_t)handle == 2u);
    return TRUE;
}

static void check_borrow(madopilot_str_t view, const wchar_t* expected)
{
    char bytes[256];
    int length = WideCharToMultiByte(
        CP_UTF8, WC_ERR_INVALID_CHARS, expected, -1, bytes, sizeof(bytes), NULL, NULL);
    CHECK(length > 0 && view.len == (size_t)length - 1u);
    for (size_t index = 0; index < live; ++index) {
        if (allocations[index].pointer == view.data) {
            CHECK(allocations[index].bytes == (size_t)length);
            CHECK(memcmp(view.data, bytes, (size_t)length) == 0);
            return;
        }
    }
    CHECK(0); /* Borrowed bytes must still belong to the entry/canonical owner. */
}

static madopilot_status_t test_describe(madopilot_build_info_t* build)
{
    (void)build;
    return MADOPILOT_STATUS_OK;
}

static madopilot_status_t test_default_create(
    const madopilot_source_t* source, const madopilot_engine_options_t* options,
    const madopilot_default_ocr_options_t* ocr, const madopilot_operation_t* operation,
    madopilot_engine_t** engine, madopilot_error_t** error)
{
    (void)source; (void)options; (void)operation;
    check_borrow(ocr->model_root, selected_paths[0]);
    check_borrow(ocr->runtime_path, selected_paths[1]);
    ++borrow_calls;
    *engine = NULL;
    *error = NULL;
    return MADOPILOT_STATUS_UNSUPPORTED;
}

static madopilot_status_t test_profile_create(
    const madopilot_source_t* source, const madopilot_engine_options_t* options,
    const madopilot_ocr_profile_options_t* profile, const madopilot_operation_t* operation,
    madopilot_engine_t** engine, madopilot_error_t** error)
{
    (void)source; (void)options; (void)operation;
    check_borrow(profile->model_root, selected_paths[0]);
    check_borrow(profile->runtime_path, selected_paths[1]);
    ++borrow_calls;
    *engine = NULL;
    *error = NULL;
    return MADOPILOT_STATUS_UNSUPPORTED;
}

static madopilot_status_t test_get_api(uint32_t major, uint32_t minor,
                                      size_t size, const madopilot_api_t** output)
{
    (void)major; (void)minor; (void)size;
    ++api_calls;
    for (int index = 0; index < expected_count; ++index) {
        CHECK(owns_text(expected_arguments[index]));
    }
    if (TEST_SAMPLE == 4) CHECK(path_calls == 2);
    if ((TEST_SAMPLE == 3 || TEST_SAMPLE == 4) && negotiate_ocr) {
        static madopilot_api_t table;
        table.struct_size = (uint32_t)sizeof(table);
        table.describe_build = test_describe;
        table.engine_create_with_default_ocr = test_default_create;
        table.engine_create_with_ocr_profile = test_profile_create;
        *output = &table;
        return MADOPILOT_STATUS_OK;
    }
    *output = NULL;
    return MADOPILOT_STATUS_UNSUPPORTED; /* Stop before any native operation. */
}

#define malloc test_malloc
#define calloc test_calloc
#define free test_free
#define WideCharToMultiByte test_convert
#define _wgetenv test_getenv
#define CreateFileW test_open
#define GetFinalPathNameByHandleW test_final_path
#define CloseHandle test_close
#define madopilot_get_api test_get_api
#define wmain ingress_wmain
#if TEST_SAMPLE == 2
/* Declare the private export locally, without importing a fixture DLL. */
#undef MADOPILOT_EXPORT
#define MADOPILOT_EXPORT
#endif
#if TEST_SAMPLE == 1
#include "../../examples/c/deterministic-slice.c"
#elif TEST_SAMPLE == 2
#include "../../examples/c/ocr-fixture.c"
#elif TEST_SAMPLE == 3
#include "../../examples/c/ocr-default.c"
#elif TEST_SAMPLE == 4
#include "../../examples/c/ocr-profile-zones.c"
#elif TEST_SAMPLE == 5
#include "../../examples/c/windows-native-input.c"
#else
#include "../../examples/c/windows-utf8-entry.h"
#endif
#undef main
#undef wmain
#undef madopilot_get_api
#undef malloc
#undef calloc
#undef free
#undef WideCharToMultiByte
#undef _wgetenv
#undef CreateFileW
#undef GetFinalPathNameByHandleW
#undef CloseHandle

#if TEST_SAMPLE == 2
madopilot_status_t madopilot_fixture_engine_create(
    const madopilot_source_t* source, const madopilot_engine_options_t* options,
    const madopilot_operation_t* operation, madopilot_engine_t** engine,
    madopilot_error_t** error)
{
    (void)source; (void)options; (void)operation; (void)engine; (void)error;
    CHECK(0); /* Negotiation must stop this sample before the private export. */
    return MADOPILOT_STATUS_UNSUPPORTED;
}
#endif

static int test_body(int argc, char** argv)
{
    ++body_calls;
    CHECK(argc == expected_count && argv[argc] == NULL);
    CHECK(live == (size_t)argc + 1u);
    for (int index = 0; index < argc; ++index) {
        CHECK(strcmp(argv[index], expected_arguments[index]) == 0);
        CHECK(owns_text(argv[index]));
    }
    return body_result;
}

#if TEST_SAMPLE == 0
static int sample_main(int argc, char** argv) { return test_body(argc, argv); }
#endif

static void reset(void)
{
    CHECK(live == 0);
    allocation_calls = fail_allocation = conversion_calls = fail_conversion = 0;
    body_calls = api_calls = path_calls = borrow_calls = 0;
    negotiate_ocr = 1;
}

static void check_conversion(void)
{
    wchar_t* wide[] = {L"program", L"", unicode_path};
    const char* expected[] = {"program", "", unicode_utf8};
    expected_arguments = expected;
    expected_count = 3;
    for (int result = 0; result <= 2; ++result) {
        reset();
        body_result = result;
        CHECK(sample_run_utf8(3, wide, test_body) == result);
        CHECK(body_calls == 1 && live == 0 && allocation_calls == 4);
    }
    for (size_t failure = 1; failure <= 4; ++failure) {
        reset();
        fail_allocation = failure;
        CHECK(sample_run_utf8(3, wide, test_body) == 1);
        CHECK(body_calls == 0 && live == 0);
    }
    for (size_t failure = 1; failure <= 6; ++failure) {
        reset();
        fail_conversion = failure;
        CHECK(sample_run_utf8(3, wide, test_body) == 1);
        CHECK(body_calls == 0 && live == 0);
    }
    wchar_t low[] = {0xdc00, 0};
    wchar_t broken_pair[] = {0xd800, L'x', 0};
    wchar_t* invalid[] = {malformed, low, broken_pair, NULL};
    for (size_t index = 0; index < 4; ++index) {
        reset();
        wide[2] = invalid[index];
        CHECK(sample_run_utf8(3, wide, test_body) == 1);
        CHECK(body_calls == 0 && live == 0);
    }
    reset();
    CHECK(sample_run_utf8(0, wide, test_body) == 1);
    CHECK(sample_run_utf8(1, NULL, test_body) == 1);
    CHECK(live == 0 && body_calls == 0);
}

#if TEST_SAMPLE != 0
static void check_entry(int argc, wchar_t** wide, const char* const* expected)
{
    reset();
    expected_arguments = expected;
    expected_count = argc;
    CHECK(ingress_wmain(argc, wide) == 1);
    CHECK(api_calls != 0 && live == 0);
    if (TEST_SAMPLE == 3 || TEST_SAMPLE == 4) CHECK(borrow_calls == 1);
    size_t allocations_needed = allocation_calls;
    size_t conversions_needed = conversion_calls;
    reset();
    negotiate_ocr = 0;
    CHECK(ingress_wmain(argc, wide) == 1);
    CHECK(api_calls != 0 && borrow_calls == 0 && live == 0);
    for (size_t failure = 1; failure <= allocations_needed; ++failure) {
        reset();
        fail_allocation = failure;
        CHECK(ingress_wmain(argc, wide) == 1);
        CHECK(api_calls == 0 && live == 0);
    }
    for (size_t failure = 1; failure <= conversions_needed; ++failure) {
        reset();
        fail_conversion = failure;
        CHECK(ingress_wmain(argc, wide) == 1);
        CHECK(api_calls == 0 && live == 0);
    }
    wchar_t* saved = wide[argc - 1];
    wide[argc - 1] = malformed;
    reset();
    CHECK(ingress_wmain(argc, wide) == 1);
    CHECK(api_calls == 0 && live == 0);
    wide[argc - 1] = saved;
}
#endif

int main(int argc, char** argv)
{
    if (argc == 2 && strcmp(argv[1], "--require-legacy-acp") == 0) {
        UINT page = GetACP();
        BOOL substituted = FALSE;
        char encoded[256];
        if (page == CP_UTF8 ||
            WideCharToMultiByte(page, WC_NO_BEST_FIT_CHARS, unicode_path, -1,
                                encoded, sizeof(encoded), NULL, &substituted) == 0 ||
            !substituted) {
            fputs("non-UTF-8 ACP with unrepresentable test paths is required\n", stderr);
            return 77;
        }
        printf("windows-path-acp=%u\n", page);
        return 0;
    }
    CHECK(argc == 1);
    check_conversion();
#if TEST_SAMPLE == 1 || TEST_SAMPLE == 2
    wchar_t* arguments[] = {L"program", L"--package", unicode_path,
                            L"--label", unicode_path};
    const char* expected[] = {"program", "--package", unicode_utf8,
                              "--label", unicode_utf8};
    check_entry(TEST_SAMPLE == 1 ? 5 : 3, arguments, expected);
#elif TEST_SAMPLE == 3 || TEST_SAMPLE == 4
    wchar_t* arguments[] = {L"program", L"--model-root", unicode_path,
                            L"--runtime", unicode_path};
    const char* expected[] = {"program", "--model-root", unicode_utf8,
                              "--runtime", unicode_utf8};
    selected_paths[0] = selected_paths[1] = unicode_path;
    environment_model = environment_runtime = malformed;
    check_entry(5, arguments, expected); /* Explicit args ignore malformed env. */
#if TEST_SAMPLE == 4
    environment_model = unicode_path;
    environment_runtime = runtime_path;
    selected_paths[1] = runtime_path;
    check_entry(1, arguments, expected);
    check_entry(3, arguments, expected); /* Model argument, runtime environment. */
    arguments[1] = L"--runtime";
    expected[1] = "--runtime";
    selected_paths[1] = unicode_path;
    check_entry(3, arguments, expected); /* Runtime argument, model environment. */
    for (int missing = 0; missing < 3; ++missing) {
        reset();
        environment_model = missing == 1 ? unicode_path : NULL;
        environment_runtime = missing == 2 ? unicode_path : NULL;
        CHECK(ingress_wmain(1, arguments) == 77);
        CHECK(api_calls == 0 && live == 0);
    }
    for (int field = 0; field < 2; ++field) {
        reset();
        environment_model = field == 0 ? malformed : unicode_path;
        environment_runtime = field == 1 ? malformed : unicode_path;
        CHECK(ingress_wmain(1, arguments) == 1);
        CHECK(api_calls == 0 && live == 0);
    }
#endif
#elif TEST_SAMPLE == 5
    wchar_t* arguments[] = {L"program", L"--extended-sequence", L"--ordinary", unicode_path};
    const char* expected[] = {"program", "--extended-sequence", "--ordinary", unicode_utf8};
    check_entry(4, arguments, expected);
    check_entry(2, arguments + 2, expected + 2);
#endif
    CHECK(live == 0);
    printf("windows-ingress-%d complete\n", TEST_SAMPLE);
    return 0;
}
