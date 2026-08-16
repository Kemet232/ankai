#!/bin/sh
set -eu

# Verify a completed ANKAI libmpv bundle without consulting a package manager.
# This checks package placement, attestation/receipt consistency, dependency
# closure, rpaths/install names, and an actual loader probe on the host OS.

die() {
    printf '%s\n' "verify-libmpv-bundle: $*" >&2
    exit 1
}

need_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

absolute_existing_path() {
    path=$1
    [ -e "$path" ] || die "path does not exist: $path"
    directory=$(dirname "$path")
    filename=$(basename "$path")
    directory=$(cd "$directory" && pwd -P) || exit 1
    printf '%s/%s\n' "$directory" "$filename"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print tolower($1) }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print tolower($1) }'
    else
        die "sha256sum or shasum is required"
    fi
}

manifest_value() {
    key=$1
    file=$2
    count=$(awk -F= -v wanted="$key" '$1 == wanted { count += 1 } END { print count + 0 }' "$file")
    [ "$count" -eq 1 ] || die "$file must contain exactly one $key entry"
    awk -v wanted="$key" 'index($0, wanted "=") == 1 { sub(/^[^=]*=/, ""); sub(/\r$/, ""); print; exit }' "$file"
}

loader_probe() {
    need_command cc
    script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
    probe_binary=$temporary_dir/libmpv-loader-probe
    case "$platform" in
        Linux) cc "$script_dir/libmpv-loader-probe.c" -o "$probe_binary" -ldl ;;
        *) cc "$script_dir/libmpv-loader-probe.c" -o "$probe_binary" ;;
    esac
    case "$platform" in
        Darwin) "$probe_binary" "$main_library" ;;
        Linux) LD_LIBRARY_PATH=$runtime_dir "$probe_binary" "$main_library" ;;
        MINGW*|MSYS*|CYGWIN*) "$probe_binary" "$main_library" ;;
    esac
}

verify_macos() {
    need_command otool
    need_command codesign
    need_command lipo
    executable_arches=$(lipo -archs "$executable")
    for runtime_file in "$runtime_dir"/*; do
        [ -f "$runtime_file" ] || continue
        runtime_arches=$(lipo -archs "$runtime_file")
        for executable_arch in $executable_arches; do
            case " $runtime_arches " in
                *" $executable_arch "*) ;;
                *) die "$runtime_file lacks executable architecture $executable_arch" ;;
            esac
        done
        codesign --verify --strict "$runtime_file" >/dev/null 2>&1 ||
            die "invalid or missing code signature after install-name rewriting: $runtime_file"
        if ! otool -l "$runtime_file" | awk '
            $1 == "cmd" && $2 == "LC_RPATH" { in_rpath = 1; next }
            in_rpath && $1 == "path" { if ($2 == "@loader_path") found = 1; in_rpath = 0 }
            END { exit found ? 0 : 1 }
        '; then
            die "bundled library lacks @loader_path rpath: $runtime_file"
        fi
        owner_id=$(otool -D "$runtime_file" 2>/dev/null | sed -n '2p')
        dependency_file=$temporary_dir/macos-dependencies
        otool -L "$runtime_file" | sed -n '2,$p' |
            sed 's/^[[:space:]]*//; s/[[:space:]]*(compatibility version.*$//' > "$dependency_file"
        while IFS= read -r dependency; do
            [ -n "$dependency" ] || continue
            [ "$dependency" = "$owner_id" ] && continue
            case "$dependency" in
                /System/Library/*|/usr/lib/*) ;;
                @rpath/*)
                    dependency_name=${dependency#@rpath/}
                    case "$dependency_name" in */*) die "nested @rpath dependency is not supported: $dependency" ;; esac
                    [ -f "$runtime_dir/$dependency_name" ] ||
                        die "bundled dependency is missing: $dependency required by $runtime_file"
                    ;;
                *) die "external macOS dependency escaped the bundle: $dependency required by $runtime_file" ;;
            esac
        done < "$dependency_file"
    done
}

is_linux_system_dependency() {
    case "$1" in
        linux-vdso.so.*|ld-linux*.so.*|ld-musl-*.so.*|libc.so.*|libm.so.*|\
        libdl.so.*|libpthread.so.*|librt.so.*|libanl.so.*|libresolv.so.*|\
        libutil.so.*|libgcc_s.so.*|libGL.so.*|libGLX.so.*|libOpenGL.so.*|\
        libEGL.so.*|libGLESv2.so.*|libvulkan.so.*|libdrm.so.*)
            return 0
            ;;
        *) return 1 ;;
    esac
}

verify_linux() {
    need_command ldd
    need_command patchelf
    for runtime_file in "$runtime_dir"/*; do
        [ -f "$runtime_file" ] || continue
        rpath=$(patchelf --print-rpath "$runtime_file")
        [ "$rpath" = '$ORIGIN' ] || die "unexpected Linux rpath '$rpath' in $runtime_file"
        ldd_output=$temporary_dir/linux-ldd
        LD_LIBRARY_PATH=$runtime_dir ldd "$runtime_file" > "$ldd_output" 2>&1 || {
            sed -n '1,120p' "$ldd_output" >&2
            die "ldd failed for bundled library: $runtime_file"
        }
        if grep -F 'not found' "$ldd_output" >/dev/null 2>&1; then
            sed -n '1,120p' "$ldd_output" >&2
            die "bundled Linux dependency is missing"
        fi
        awk '/=>/ { print $1 "|" $3 }' "$ldd_output" > "$temporary_dir/linux-resolved"
        while IFS='|' read -r dependency_name resolved; do
            [ -n "$dependency_name" ] || continue
            if is_linux_system_dependency "$dependency_name"; then
                continue
            fi
            case "$resolved" in
                "$runtime_dir"/*) ;;
                *) die "non-system Linux dependency resolved outside the bundle: $dependency_name => $resolved" ;;
            esac
        done < "$temporary_dir/linux-resolved"
    done
}

is_windows_system_dependency() {
    normalized=$(printf '%s' "$1" | tr '[:lower:]' '[:upper:]')
    case "$normalized" in
        API-MS-WIN-*.DLL|EXT-MS-WIN-*.DLL|KERNEL32.DLL|USER32.DLL|GDI32.DLL|\
        ADVAPI32.DLL|SHELL32.DLL|OLE32.DLL|OLEAUT32.DLL|COMDLG32.DLL|\
        COMCTL32.DLL|WS2_32.DLL|BCRYPT.DLL|CRYPT32.DLL|SECUR32.DLL|\
        NTDLL.DLL|RPCRT4.DLL|SHLWAPI.DLL|VERSION.DLL|WINMM.DLL|\
        IMM32.DLL|DWMAPI.DLL|DXGI.DLL|D3D11.DLL|D3D12.DLL|MF.DLL|\
        MFPLAT.DLL|MFREADWRITE.DLL|PROPSYS.DLL|UCRTBASE.DLL)
            return 0
            ;;
        *) return 1 ;;
    esac
}

verify_windows() {
    if command -v llvm-objdump >/dev/null 2>&1; then
        objdump_command=llvm-objdump
    elif command -v objdump >/dev/null 2>&1; then
        objdump_command=objdump
    else
        die "llvm-objdump or objdump is required for Windows import validation"
    fi
    for runtime_file in "$runtime_dir"/*.dll; do
        [ -f "$runtime_file" ] || continue
        imports_file=$temporary_dir/windows-imports
        "$objdump_command" -p "$runtime_file" |
            awk 'toupper($1) == "DLL" && toupper($2) == "NAME:" { print $3 }' > "$imports_file"
        [ -s "$imports_file" ] || die "could not read PE imports from $runtime_file"
        while IFS= read -r dependency; do
            [ -n "$dependency" ] || continue
            if is_windows_system_dependency "$dependency"; then
                continue
            fi
            match=$(find "$runtime_dir" -maxdepth 1 -type f -iname "$dependency" -print | sed -n '1p')
            [ -n "$match" ] || die "Windows dependency is missing beside executable: $dependency"
        done < "$imports_file"
    done
}

[ "$#" -eq 2 ] || die "usage: $0 BUNDLE_ROOT EXECUTABLE"
destination=$(absolute_existing_path "$1")
executable=$(absolute_existing_path "$2")
[ -d "$destination" ] || die "bundle root is not a directory: $destination"
[ -f "$executable" ] || die "ANKAI executable is not a regular file: $executable"
case "$executable" in "$destination"/*) ;; *) die "executable must be inside bundle root" ;; esac

platform=$(uname -s)
case "$platform" in
    Darwin)
        runtime_dir=$destination/Contents/Frameworks
        license_dir=$destination/Contents/Resources/licenses/libmpv-runtime
        main_library=$runtime_dir/libmpv.dylib
        expected_executable_dir=$destination/Contents/MacOS
        executable_marker=../Frameworks/libmpv.dylib
        ;;
    Linux)
        runtime_dir=$destination/lib
        license_dir=$destination/share/licenses/libmpv-runtime
        main_library=$runtime_dir/libmpv.so.2
        expected_executable_dir=$destination/bin
        executable_marker=../lib/libmpv.so.2
        ;;
    MINGW*|MSYS*|CYGWIN*)
        runtime_dir=$(dirname "$executable")
        license_dir=$destination/licenses/libmpv-runtime
        main_library=$runtime_dir/mpv-2.dll
        expected_executable_dir=$destination
        executable_marker=mpv-2.dll
        ;;
    *) die "unsupported verification platform: $platform" ;;
esac

[ "$(dirname "$executable")" = "$expected_executable_dir" ] ||
    die "executable is not in the platform's expected bundle location"
[ -f "$main_library" ] || die "bundled libmpv is missing: $main_library"
[ -d "$license_dir/upstream-license-inventory" ] || die "upstream license inventory is missing"
first_license=$(find "$license_dir/upstream-license-inventory" -type f -print | sed -n '1p')
[ -n "$first_license" ] || die "upstream license inventory is empty"
attestation=$license_dir/BUILD-ATTESTATION.env
receipt=$license_dir/BUNDLE-RECEIPT.env
[ -f "$attestation" ] || die "build attestation is missing from bundle"
[ -f "$receipt" ] || die "bundle receipt is missing"

[ "$(manifest_value ANKAI_LIBMPV_ATTESTATION "$attestation")" = "1" ] || die "unsupported attestation format"
[ "$(manifest_value MPV_GPL "$attestation")" = "false" ] || die "bundle is not attested as an LGPL mpv build"
[ "$(manifest_value MPV_SHARED "$attestation")" = "true" ] || die "bundle is not attested as a shared build"
[ "$(manifest_value FFMPEG_GPL "$attestation")" = "false" ] || die "FFmpeg GPL features are not explicitly disabled"
[ "$(manifest_value FFMPEG_NONFREE "$attestation")" = "false" ] || die "FFmpeg nonfree features are not explicitly disabled"
[ "$(manifest_value ANKAI_LIBMPV_BUNDLE_RECEIPT "$receipt")" = "1" ] || die "unsupported bundle receipt format"
[ "$(manifest_value PLATFORM "$receipt")" = "$platform" ] || die "bundle was produced for a different platform"
[ "$(manifest_value BUILD_ID "$receipt")" = "$(manifest_value BUILD_ID "$attestation")" ] || die "build ID mismatch"
[ "$(manifest_value ATTESTED_UPSTREAM_SHA256 "$receipt")" = "$(manifest_value LIBMPV_SHA256 "$attestation")" ] ||
    die "receipt does not refer to the preserved attestation"
[ "$(manifest_value BUNDLED_LIBMPV_SHA256 "$receipt")" = "$(sha256_file "$main_library")" ] ||
    die "bundled libmpv hash does not match its receipt"

need_command strings
strings "$executable" | grep -F "$executable_marker" >/dev/null 2>&1 ||
    die "executable does not contain the bundled libmpv lookup path: $executable_marker"

temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/ankai-libmpv-verify.XXXXXX") || die "could not create temporary directory"
trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM
case "$platform" in
    Darwin) verify_macos ;;
    Linux) verify_linux ;;
    MINGW*|MSYS*|CYGWIN*) verify_windows ;;
esac
loader_probe
printf '%s\n' "Verified bundled libmpv dependency closure for $destination"
