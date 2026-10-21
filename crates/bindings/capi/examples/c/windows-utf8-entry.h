/* Private sample entry: Windows OS strings are owned UTF-8, never ACP bytes. */
#ifndef MADOPILOT_SAMPLE_WINDOWS_UTF8_ENTRY_H
#define MADOPILOT_SAMPLE_WINDOWS_UTF8_ENTRY_H
#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static char* sample_utf8(const wchar_t* wide)
{
    int bytes = WideCharToMultiByte(
        CP_UTF8, WC_ERR_INVALID_CHARS, wide, -1, NULL, 0, NULL, NULL);
    if (bytes == 0) return NULL;
    char* text = (char*)malloc((size_t)bytes);
    if (text == NULL) return NULL;
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide, -1,
                            text, bytes, NULL, NULL) != bytes) {
        free(text);
        return NULL;
    }
    return text;
}

static int sample_run_utf8(int argc, wchar_t** wide,
                           int (*run)(int, char**))
{
    int result = 1;
    char** arguments = NULL;
    if (argc < 1 || wide == NULL ||
        (size_t)argc > SIZE_MAX / sizeof(*arguments) - 1u) goto invalid;
    arguments = (char**)calloc((size_t)argc + 1u, sizeof(*arguments));
    if (arguments == NULL) goto invalid;
    for (int index = 0; index < argc; ++index) {
        if (wide[index] == NULL) goto invalid;
        arguments[index] = sample_utf8(wide[index]);
        if (arguments[index] == NULL) goto invalid;
    }
    result = run(argc, arguments);
    goto cleanup;
invalid:
    fputs("could not convert Windows arguments to UTF-8\n", stderr);
cleanup:
    if (arguments != NULL) {
        for (int index = 0; index < argc; ++index) free(arguments[index]);
        free(arguments);
    }
    return result;
}

static int sample_main(int argc, char** argv);

int wmain(int argc, wchar_t** wide)
{
    return sample_run_utf8(argc, wide, sample_main);
}

#define main sample_main
#endif
#endif
