/* Private host-load experiment; not part of the released C ABI. */
#define MADOPILOT_BUILDING
#define madopilot_get_api qualification_get_api
#include <madopilot/madopilot.h>
#undef madopilot_get_api
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* OS framework images are separate from the 512-file supplied closure limit. */
#define PROFILE_IMAGE_LIMIT 4096

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
static HMODULE library;
typedef FARPROC address_t;
#else
#include <dlfcn.h>
#include <limits.h>
#include <mach-o/dyld.h>
static void* library;
typedef void* address_t;
#endif

static void report_modules(void)
{
#ifdef _WIN32
    HMODULE modules[PROFILE_IMAGE_LIMIT];
    DWORD needed = 0;
    if (!EnumProcessModules(GetCurrentProcess(), modules, sizeof(modules), &needed) ||
        needed > sizeof(modules)) {
        puts("MADO_PROFILE_MODULES=incomplete");
        return;
    }
    for (DWORD index = 0; index < needed / sizeof(HMODULE); ++index) {
        wchar_t path[32768];
        char utf8[131072];
        DWORD count = GetModuleFileNameExW(GetCurrentProcess(), modules[index], path, 32768);
        if (count == 0 || count >= 32768 ||
            WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, path, -1, utf8,
                                sizeof(utf8), NULL, NULL) == 0) {
            puts("MADO_PROFILE_MODULES=incomplete");
            return;
        }
        printf("MADO_PROFILE_MODULE=%s\n", utf8);
    }
#else
    uint32_t count = _dyld_image_count();
    if (count > PROFILE_IMAGE_LIMIT) {
        puts("MADO_PROFILE_MODULES=incomplete");
        return;
    }
    for (uint32_t index = 0; index < count; ++index) {
        const char* path = _dyld_get_image_name(index);
        if (path == NULL) {
            puts("MADO_PROFILE_MODULES=incomplete");
            return;
        }
        printf("MADO_PROFILE_MODULE=%s\n", path);
    }
#endif
    puts("MADO_PROFILE_MODULES=complete");
}

static int load_candidate(void)
{
    if (library != NULL) return 1;
#ifdef _WIN32
    wchar_t wide[32768];
    wchar_t full[32768];
    DWORD supplied = GetEnvironmentVariableW(L"MADO_PROFILE_LIBRARY", wide, 32768);
    if (supplied < 3 || supplied >= 32768 ||
        !((wide[1] == L':' && (wide[2] == L'/' || wide[2] == L'\\')) ||
          (wide[0] == L'\\' && wide[1] == L'\\'))) {
        puts("MADO_PROFILE_LOAD=invalid-path");
        return 0;
    }
    DWORD length = GetFullPathNameW(wide, 32768, full, NULL);
    if (length == 0 || length >= 32768) {
        puts("MADO_PROFILE_LOAD=invalid-path");
        return 0;
    }
    library = LoadLibraryExW(full, NULL,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
#else
    const char* path = getenv("MADO_PROFILE_LIBRARY");
    char canonical[PATH_MAX];
    if (path == NULL || path[0] != '/') {
        puts("MADO_PROFILE_LOAD=invalid-path");
        return 0;
    }
    if (realpath(path, canonical) == NULL) {
        puts("MADO_PROFILE_LOAD=unavailable");
        return 0;
    }
    library = dlopen(canonical, RTLD_NOW | RTLD_LOCAL);
#endif
    if (library == NULL) {
        puts("MADO_PROFILE_LOAD=unavailable");
        return 0;
    }
    /* Process-long API/ORT pointers forbid unloading the prototype underneath them. */
    if (atexit(report_modules) != 0) {
        puts("MADO_PROFILE_LOAD=observer-failed");
        return 0;
    }
    puts("MADO_PROFILE_LOAD=loaded");
    return 1;
}

static address_t symbol(const char* name)
{
#ifdef _WIN32
    return GetProcAddress(library, name);
#else
    return dlsym(library, name);
#endif
}

madopilot_status_t qualification_get_api(uint32_t major, uint32_t minor, size_t extent,
                                        const madopilot_api_t** output)
{
    typedef madopilot_status_t (*getter_t)(uint32_t, uint32_t, size_t, const madopilot_api_t**);
    if (output == NULL) return MADOPILOT_STATUS_INVALID_ARGUMENT;
    *output = NULL;
    if (!load_candidate()) return MADOPILOT_STATUS_UNSUPPORTED;
    address_t address = symbol("madopilot_get_api");
    getter_t getter = NULL;
    _Static_assert(sizeof(getter) == sizeof(address), "native function pointer size");
    memcpy(&getter, &address, sizeof(getter));
    if (getter == NULL) {
        puts("MADO_PROFILE_LOAD=incompatible");
        return MADOPILOT_STATUS_UNSUPPORTED;
    }
    return getter(major, minor, extent, output);
}

#ifdef PROFILE_RUST_MAIN
int main(void)
{
    typedef int (*probe_t)(void);
    if (!load_candidate()) return 1;
    address_t address = symbol("mado_profile_rust_probe");
    probe_t probe = NULL;
    _Static_assert(sizeof(probe) == sizeof(address), "native function pointer size");
    memcpy(&probe, &address, sizeof(probe));
    if (probe == NULL) {
        puts("MADO_PROFILE_LOAD=incompatible");
        return 1;
    }
    return probe();
}
#endif
