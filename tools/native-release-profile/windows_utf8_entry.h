/* Private forced include: keep frozen consumers unchanged while Windows CRT supplies UTF-16. */
#ifndef MADOPILOT_PROFILE_UTF8_ENTRY_H
#define MADOPILOT_PROFILE_UTF8_ENTRY_H
#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>

static int profile_main(int argc, char** argv);

int wmain(int argc, wchar_t** wide)
{
    int result = 1;
    char** arguments = NULL;
    if (argc < 1) goto invalid;
    arguments = (char**)calloc((size_t)argc + 1u, sizeof(*arguments));
    if (arguments == NULL) goto invalid;
    for (int at = 0; at < argc; ++at) {
        int bytes = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide[at], -1,
                                        NULL, 0, NULL, NULL);
        if (bytes == 0) goto invalid;
        arguments[at] = (char*)malloc((size_t)bytes);
        if (arguments[at] == NULL ||
            WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide[at], -1,
                                arguments[at], bytes, NULL, NULL) == 0) goto invalid;
    }
    result = profile_main(argc, arguments);
    goto done;
invalid:
    fputs("MADO_PROFILE_FAILURE=argv-utf8\n", stderr);
done:
    if (arguments != NULL) {
        for (int at = 0; at < argc; ++at) free(arguments[at]);
        free(arguments);
    }
    return result;
}

#define main profile_main
#endif
#endif
