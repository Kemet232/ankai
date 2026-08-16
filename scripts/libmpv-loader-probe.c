/* Minimal release-verification helper. It loads the exact bundled libmpv path
 * and resolves one stable API symbol; it never initializes playback. */
#include <stdio.h>

#if defined(_WIN32)
#include <windows.h>
#ifndef LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
#define LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR 0x00000100
#endif
#ifndef LOAD_LIBRARY_SEARCH_DEFAULT_DIRS
#define LOAD_LIBRARY_SEARCH_DEFAULT_DIRS 0x00001000
#endif
#else
#include <dlfcn.h>
#endif

int main(int argc, char **argv) {
    if (argc != 2) {
        fputs("usage: libmpv-loader-probe LIBMPV_PATH\n", stderr);
        return 64;
    }

#if defined(_WIN32)
    HMODULE library = LoadLibraryExA(
        argv[1], NULL,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (library == NULL) {
        fprintf(stderr, "LoadLibrary failed for bundled libmpv (error %lu)\n",
                (unsigned long)GetLastError());
        return 1;
    }
    if (GetProcAddress(library, "mpv_client_api_version") == NULL) {
        fputs("bundled DLL does not export mpv_client_api_version\n", stderr);
        FreeLibrary(library);
        return 1;
    }
    FreeLibrary(library);
#else
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (library == NULL) {
        fprintf(stderr, "dlopen failed for bundled libmpv: %s\n", dlerror());
        return 1;
    }
    (void)dlerror();
    if (dlsym(library, "mpv_client_api_version") == NULL) {
        fprintf(stderr, "bundled library does not export mpv_client_api_version: %s\n",
                dlerror());
        dlclose(library);
        return 1;
    }
    dlclose(library);
#endif
    return 0;
}
