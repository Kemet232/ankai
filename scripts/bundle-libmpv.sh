#!/bin/sh
set -eu

# Assemble ANKAI's redistributable libmpv runtime. This script deliberately
# requires a build attestation: an ordinary/default mpv build is GPL and must
# never be copied into an ANKAI release by accident.
#
# Usage:
#   scripts/bundle-libmpv.sh BUNDLE_ROOT PREFIX ATTESTATION EXECUTABLE [LICENSE_DIR]
#
# BUNDLE_ROOT is the .app on macOS and the release root on Linux/Windows.
# PREFIX is an installed, shared, LGPL-configured mpv/FFmpeg prefix.
# EXECUTABLE is the final ANKAI executable inside BUNDLE_ROOT.
# LICENSE_DIR defaults to PREFIX/share/licenses and must contain the complete
# license inventory produced by the dependency build pipeline.

die() {
    printf '%s\n' "bundle-libmpv: $*" >&2
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
    [ "$count" -eq 1 ] || die "attestation must contain exactly one $key entry"
    awk -v wanted="$key" 'index($0, wanted "=") == 1 { sub(/^[^=]*=/, ""); sub(/\r$/, ""); print; exit }' "$file"
}

validate_attestation() {
    attestation=$1
    source_library=$2

    awk '
        /^[[:space:]]*$/ || /^#/ { next }
        !/^[A-Z][A-Z0-9_]*=/ { bad = 1; next }
        {
            key = $0
            sub(/=.*/, "", key)
            if (key != "ANKAI_LIBMPV_ATTESTATION" &&
                key != "MPV_GPL" && key != "MPV_SHARED" &&
                key != "FFMPEG_GPL" && key != "FFMPEG_NONFREE" &&
                key != "LIBMPV_SHA256" && key != "BUILD_ID" &&
                key != "CORRESPONDING_SOURCE_URL") bad = 1
        }
        END { exit bad ? 1 : 0 }
    ' "$attestation" || die "attestation contains malformed or unknown fields"

    [ "$(manifest_value ANKAI_LIBMPV_ATTESTATION "$attestation")" = "1" ] ||
        die "unsupported libmpv attestation format"
    [ "$(manifest_value MPV_GPL "$attestation")" = "false" ] ||
        die "refusing libmpv: MPV_GPL must be explicitly false"
    [ "$(manifest_value MPV_SHARED "$attestation")" = "true" ] ||
        die "refusing libmpv: MPV_SHARED must be explicitly true"
    [ "$(manifest_value FFMPEG_GPL "$attestation")" = "false" ] ||
        die "refusing libmpv: FFMPEG_GPL must be explicitly false"
    [ "$(manifest_value FFMPEG_NONFREE "$attestation")" = "false" ] ||
        die "refusing libmpv: FFMPEG_NONFREE must be explicitly false"

    expected_hash=$(manifest_value LIBMPV_SHA256 "$attestation")
    case "$expected_hash" in
        *[!0-9A-Fa-f]*|'') die "LIBMPV_SHA256 is not a hexadecimal SHA-256" ;;
    esac
    [ "${#expected_hash}" -eq 64 ] || die "LIBMPV_SHA256 must contain 64 hexadecimal characters"
    actual_hash=$(sha256_file "$source_library")
    [ "$(printf '%s' "$expected_hash" | tr 'A-F' 'a-f')" = "$actual_hash" ] ||
        die "libmpv binary does not match the attested SHA-256"

    build_id=$(manifest_value BUILD_ID "$attestation")
    [ -n "$build_id" ] || die "BUILD_ID must not be empty"
    source_url=$(manifest_value CORRESPONDING_SOURCE_URL "$attestation")
    case "$source_url" in
        https://*) ;;
        *) die "CORRESPONDING_SOURCE_URL must be an explicit HTTPS URL" ;;
    esac
}

reject_unsafe_queue_path() {
    case "$1" in
        *'|'*) die "dependency path contains unsupported character '|': $1" ;;
    esac
}

add_runtime_file() {
    source_file=$1
    target_name=$2
    reject_unsafe_queue_path "$source_file"
    reject_unsafe_queue_path "$target_name"
    case "$target_name" in
        */*|'') die "invalid runtime filename: $target_name" ;;
    esac
    [ -f "$source_file" ] || die "runtime dependency is not a regular file: $source_file"

    existing_source=$(awk -F'|' -v wanted="$target_name" '$1 == wanted { print substr($0, length($1) + 2); exit }' "$mapping_file")
    if [ -n "$existing_source" ]; then
        cmp -s "$existing_source" "$source_file" ||
            die "dependency filename collision for $target_name: $existing_source and $source_file"
        return
    fi

    cp -L "$source_file" "$stage_runtime/$target_name"
    printf '%s|%s\n' "$target_name" "$source_file" >> "$mapping_file"
    printf '%s|%s\n' "$source_file" "$target_name" >> "$queue_file"
}

is_macos_system_dependency() {
    case "$1" in
        /System/Library/*|/usr/lib/*) return 0 ;;
        *) return 1 ;;
    esac
}

resolve_macos_dependency() {
    dependency=$1
    owner=$2
    owner_dir=$(dirname "$owner")
    case "$dependency" in
        /*)
            candidate=$dependency
            ;;
        @loader_path/*)
            candidate=$owner_dir/${dependency#@loader_path/}
            ;;
        @rpath/*)
            suffix=${dependency#@rpath/}
            candidate=
            for search_root in "$owner_dir" "$prefix/lib" "$prefix/bin"; do
                if [ -f "$search_root/$suffix" ]; then
                    candidate=$search_root/$suffix
                    break
                fi
            done
            [ -n "$candidate" ] || die "cannot resolve $dependency required by $owner"
            ;;
        @executable_path/*)
            candidate=$prefix/bin/${dependency#@executable_path/}
            ;;
        *)
            die "unsupported macOS install name '$dependency' required by $owner"
            ;;
    esac
    [ -f "$candidate" ] || die "missing macOS dependency $dependency (resolved as $candidate)"
    printf '%s\n' "$candidate"
}

bundle_macos() {
    need_command otool
    need_command install_name_tool
    need_command codesign

    add_runtime_file "$source_library" libmpv.dylib
    queue_index=1
    while :; do
        queue_entry=$(sed -n "${queue_index}p" "$queue_file")
        [ -n "$queue_entry" ] || break
        owner=${queue_entry%%|*}
        target=${queue_entry#*|}
        staged_owner=$stage_runtime/$target
        owner_id=$(otool -D "$owner" 2>/dev/null | sed -n '2p')

        dependency_file=$stage_dir/macos-dependencies
        otool -L "$owner" | sed -n '2,$p' |
            sed 's/^[[:space:]]*//; s/[[:space:]]*(compatibility version.*$//' > "$dependency_file"
        while IFS= read -r dependency; do
            [ -n "$dependency" ] || continue
            [ "$dependency" = "$owner_id" ] && continue
            if is_macos_system_dependency "$dependency"; then
                continue
            fi
            case "$dependency" in
                *.framework/*)
                    die "non-system framework dependency requires an explicit packaging policy: $dependency"
                    ;;
            esac
            resolved=$(resolve_macos_dependency "$dependency" "$owner")
            dependency_name=$(basename "$dependency")
            add_runtime_file "$resolved" "$dependency_name"
            install_name_tool -change "$dependency" "@rpath/$dependency_name" "$staged_owner"
        done < "$dependency_file"

        install_name_tool -id "@rpath/$target" "$staged_owner"
        if ! otool -l "$staged_owner" | awk '
            $1 == "cmd" && $2 == "LC_RPATH" { in_rpath = 1; next }
            in_rpath && $1 == "path" { if ($2 == "@loader_path") found = 1; in_rpath = 0 }
            END { exit found ? 0 : 1 }
        '; then
            install_name_tool -add_rpath @loader_path "$staged_owner"
        fi
        queue_index=$((queue_index + 1))
    done

    for runtime_file in "$stage_runtime"/*; do
        codesign --force --sign - "$runtime_file" >/dev/null
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

bundle_linux() {
    need_command ldd
    need_command readelf
    need_command patchelf

    add_runtime_file "$source_library" libmpv.so.2
    queue_index=1
    while :; do
        queue_entry=$(sed -n "${queue_index}p" "$queue_file")
        [ -n "$queue_entry" ] || break
        owner=${queue_entry%%|*}
        target=${queue_entry#*|}
        staged_owner=$stage_runtime/$target

        ldd_output=$stage_dir/linux-ldd
        ldd "$owner" > "$ldd_output" 2>&1 || {
            sed -n '1,120p' "$ldd_output" >&2
            die "ldd failed for $owner"
        }
        if grep -F 'not found' "$ldd_output" >/dev/null 2>&1; then
            sed -n '1,120p' "$ldd_output" >&2
            die "unresolved dependency in source prefix: $owner"
        fi
        parsed_dependencies=$stage_dir/linux-dependencies
        awk '
            /=>/ && $3 ~ /^\// { print $1 "|" $3; next }
            $1 ~ /^\// { n = split($1, parts, "/"); print parts[n] "|" $1 }
        ' "$ldd_output" > "$parsed_dependencies"
        while IFS='|' read -r dependency_name resolved; do
            [ -n "$dependency_name" ] || continue
            if is_linux_system_dependency "$dependency_name"; then
                continue
            fi
            [ -f "$resolved" ] || die "missing Linux dependency $dependency_name for $owner"
            add_runtime_file "$resolved" "$dependency_name"
        done < "$parsed_dependencies"

        patchelf --set-rpath '$ORIGIN' "$staged_owner"
        queue_index=$((queue_index + 1))
    done

    soname=$(readelf -d "$stage_runtime/libmpv.so.2" |
        awk '/\(SONAME\)/ { value = $NF; gsub(/^\[|\]$/, "", value); print value; exit }')
    [ "$soname" = "libmpv.so.2" ] || die "unexpected libmpv SONAME after packaging: ${soname:-missing}"
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

windows_imports() {
    binary=$1
    "$objdump_command" -p "$binary" |
        awk 'toupper($1) == "DLL" && toupper($2) == "NAME:" { print $3 }'
}

resolve_windows_dependency() {
    dependency=$1
    match=$(find "$prefix/bin" "$prefix/lib" -maxdepth 1 -type f -iname "$dependency" -print 2>/dev/null |
        sed -n '1p')
    [ -n "$match" ] || die "Windows dependency is neither bundled nor allowlisted as an OS DLL: $dependency"
    printf '%s\n' "$match"
}

bundle_windows() {
    if command -v llvm-objdump >/dev/null 2>&1; then
        objdump_command=llvm-objdump
    elif command -v objdump >/dev/null 2>&1; then
        objdump_command=objdump
    else
        die "llvm-objdump or objdump is required for Windows import validation"
    fi

    add_runtime_file "$source_library" mpv-2.dll
    queue_index=1
    while :; do
        queue_entry=$(sed -n "${queue_index}p" "$queue_file")
        [ -n "$queue_entry" ] || break
        owner=${queue_entry%%|*}
        imports_file=$stage_dir/windows-imports
        windows_imports "$owner" > "$imports_file"
        [ -s "$imports_file" ] || die "could not read PE imports from $owner"
        while IFS= read -r dependency; do
            [ -n "$dependency" ] || continue
            if is_windows_system_dependency "$dependency"; then
                continue
            fi
            resolved=$(resolve_windows_dependency "$dependency")
            add_runtime_file "$resolved" "$dependency"
        done < "$imports_file"
        queue_index=$((queue_index + 1))
    done
}

[ "$#" -ge 4 ] && [ "$#" -le 5 ] ||
    die "usage: $0 BUNDLE_ROOT PREFIX ATTESTATION EXECUTABLE [LICENSE_DIR]"

destination=$(absolute_existing_path "$1")
prefix=$(absolute_existing_path "$2")
attestation=$(absolute_existing_path "$3")
executable=$(absolute_existing_path "$4")
license_source=${5:-$prefix/share/licenses}
license_source=$(absolute_existing_path "$license_source")

[ -d "$destination" ] || die "bundle root is not a directory: $destination"
[ -d "$prefix" ] || die "prefix is not a directory: $prefix"
[ -f "$attestation" ] || die "attestation is not a regular file: $attestation"
[ -f "$executable" ] || die "ANKAI executable is not a regular file: $executable"
[ -d "$license_source" ] || die "license inventory is not a directory: $license_source"
first_license=$(find "$license_source" -type f -print | sed -n '1p')
[ -n "$first_license" ] || die "license inventory is empty: $license_source"

case "$executable" in
    "$destination"/*) ;;
    *) die "executable must be located inside the bundle root" ;;
esac

platform=$(uname -s)
case "$platform" in
    Darwin)
        source_library=$prefix/lib/libmpv.dylib
        runtime_destination=$destination/Contents/Frameworks
        license_destination=$destination/Contents/Resources/licenses/libmpv-runtime
        expected_executable_dir=$destination/Contents/MacOS
        executable_marker=../Frameworks/libmpv.dylib
        ;;
    Linux)
        source_library=$prefix/lib/libmpv.so.2
        runtime_destination=$destination/lib
        license_destination=$destination/share/licenses/libmpv-runtime
        expected_executable_dir=$destination/bin
        executable_marker=../lib/libmpv.so.2
        ;;
    MINGW*|MSYS*|CYGWIN*)
        source_library=$prefix/bin/mpv-2.dll
        runtime_destination=$(dirname "$executable")
        license_destination=$destination/licenses/libmpv-runtime
        expected_executable_dir=$destination
        executable_marker=mpv-2.dll
        ;;
    *) die "unsupported packaging platform: $platform" ;;
esac

[ -f "$source_library" ] || die "expected libmpv runtime is missing: $source_library"
[ "$(dirname "$executable")" = "$expected_executable_dir" ] ||
    die "executable must be directly inside $expected_executable_dir on $platform"
need_command strings
strings "$executable" | grep -F "$executable_marker" >/dev/null 2>&1 ||
    die "executable does not contain ANKAI's bundled libmpv lookup path: $executable_marker"

validate_attestation "$attestation" "$source_library"

stage_dir=$(mktemp -d "${TMPDIR:-/tmp}/ankai-libmpv.XXXXXX") || die "could not create staging directory"
trap 'rm -rf "$stage_dir"' EXIT HUP INT TERM
stage_runtime=$stage_dir/runtime
stage_licenses=$stage_dir/licenses
mapping_file=$stage_dir/runtime-map
queue_file=$stage_dir/runtime-queue
mkdir -p "$stage_runtime" "$stage_licenses"
: > "$mapping_file"
: > "$queue_file"

case "$platform" in
    Darwin) bundle_macos ;;
    Linux) bundle_linux ;;
    MINGW*|MSYS*|CYGWIN*) bundle_windows ;;
esac

cp -R "$license_source"/. "$stage_licenses/upstream-license-inventory"
cp "$attestation" "$stage_licenses/BUILD-ATTESTATION.env"
case "$platform" in
    Darwin) bundled_main_hash=$(sha256_file "$stage_runtime/libmpv.dylib") ;;
    Linux) bundled_main_hash=$(sha256_file "$stage_runtime/libmpv.so.2") ;;
    MINGW*|MSYS*|CYGWIN*) bundled_main_hash=$(sha256_file "$stage_runtime/mpv-2.dll") ;;
esac
{
    printf 'ANKAI_LIBMPV_BUNDLE_RECEIPT=1\n'
    printf 'PLATFORM=%s\n' "$platform"
    printf 'BUILD_ID=%s\n' "$(manifest_value BUILD_ID "$attestation")"
    printf 'ATTESTED_UPSTREAM_SHA256=%s\n' "$(manifest_value LIBMPV_SHA256 "$attestation")"
    printf 'BUNDLED_LIBMPV_SHA256=%s\n' "$bundled_main_hash"
} > "$stage_licenses/BUNDLE-RECEIPT.env"

[ ! -e "$license_destination" ] ||
    die "license destination already exists; refusing to merge with stale output: $license_destination"
for staged_file in "$stage_runtime"/*; do
    target_file=$runtime_destination/$(basename "$staged_file")
    [ ! -e "$target_file" ] || die "runtime output already exists: $target_file"
done

mkdir -p "$runtime_destination" "$(dirname "$license_destination")"
for staged_file in "$stage_runtime"/*; do
    cp "$staged_file" "$runtime_destination/$(basename "$staged_file")"
done
cp -R "$stage_licenses" "$license_destination"

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
"$script_dir/verify-libmpv-bundle.sh" "$destination" "$executable"
printf '%s\n' "Bundled verified LGPL-attested libmpv runtime into $destination"
