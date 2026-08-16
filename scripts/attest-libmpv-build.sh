#!/bin/sh
set -eu

# Emit a machine-readable attestation for the exact installed libmpv binary.
# Run this in the trusted build job, then redirect stdout to a release input:
#
#   scripts/attest-libmpv-build.sh PREFIX MPV_BUILD_DIR FFMPEG BUILD_ID SOURCE_URL \
#       > libmpv-build.env

die() {
    printf '%s\n' "attest-libmpv-build: $*" >&2
    exit 1
}

need_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
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

[ "$#" -eq 5 ] ||
    die "usage: $0 PREFIX MPV_BUILD_DIR FFMPEG BUILD_ID CORRESPONDING_SOURCE_URL"

prefix=$1
mpv_build_dir=$2
ffmpeg=$3
build_id=$4
source_url=$5

[ -d "$prefix" ] || die "installed prefix does not exist: $prefix"
[ -d "$mpv_build_dir" ] || die "mpv Meson build directory does not exist: $mpv_build_dir"
[ -x "$ffmpeg" ] || die "FFmpeg executable does not exist or is not executable: $ffmpeg"
prefix=$(cd "$prefix" && pwd -P)
ffmpeg_directory=$(cd "$(dirname "$ffmpeg")" && pwd -P)
ffmpeg=$ffmpeg_directory/$(basename "$ffmpeg")
case "$ffmpeg" in
    "$prefix"/bin/*) ;;
    *) die "FFmpeg must come from the same isolated prefix as libmpv" ;;
esac
[ -n "$build_id" ] || die "BUILD_ID must not be empty"
case "$build_id" in
    *'='*) die "BUILD_ID contains an unsupported character" ;;
esac
case "$source_url" in
    https://*) ;;
    *) die "CORRESPONDING_SOURCE_URL must be HTTPS" ;;
esac
case "$source_url" in
    *'='*) die "CORRESPONDING_SOURCE_URL contains an unsupported character" ;;
esac

need_command meson
need_command jq
build_options=$(meson introspect "$mpv_build_dir" --buildoptions) ||
    die "could not inspect mpv Meson build options"
printf '%s' "$build_options" | jq -e '
    any(.[]; .name == "gpl" and .value == false) and
    any(.[]; .name == "libmpv" and .value == true) and
    any(.[]; .name == "default_library" and (.value == "shared" or .value == "both"))
' >/dev/null || die "mpv build is not explicitly gpl=false with a shared libmpv"

ffmpeg_configuration=$($ffmpeg -buildconf 2>&1) || die "could not read FFmpeg build configuration"
if printf '%s\n' "$ffmpeg_configuration" |
    grep -E -- '(^|[[:space:]])--enable-gpl([[:space:]]|$)' >/dev/null 2>&1; then
    die "FFmpeg was built with --enable-gpl"
fi
if printf '%s\n' "$ffmpeg_configuration" |
    grep -E -- '(^|[[:space:]])--enable-nonfree([[:space:]]|$)' >/dev/null 2>&1; then
    die "FFmpeg was built with --enable-nonfree"
fi
if ! printf '%s\n' "$ffmpeg_configuration" |
    grep -E -- '(^|[[:space:]])--disable-gpl([[:space:]]|$)' >/dev/null 2>&1; then
    die "FFmpeg build does not record the required explicit --disable-gpl flag"
fi
if ! printf '%s\n' "$ffmpeg_configuration" |
    grep -E -- '(^|[[:space:]])--disable-nonfree([[:space:]]|$)' >/dev/null 2>&1; then
    die "FFmpeg build does not record the required explicit --disable-nonfree flag"
fi

case "$(uname -s)" in
    Darwin)
        need_command otool
        library=$prefix/lib/libmpv.dylib
        [ -f "$library" ] || die "installed libmpv runtime is missing: $library"
        linked_ffmpeg=$(
            otool -L "$library" | sed -n '2,$p' |
                sed 's/^[[:space:]]*//; s/[[:space:]]*(compatibility version.*$//' |
                awk '/\/lib(avcodec|avformat|avutil)\.[0-9]+\.dylib$/ || /^@rpath\/lib(avcodec|avformat|avutil)\.[0-9]+\.dylib$/ { print }'
        )
        ;;
    Linux)
        need_command ldd
        library=$prefix/lib/libmpv.so.2
        [ -f "$library" ] || die "installed libmpv runtime is missing: $library"
        linked_ffmpeg=$(LD_LIBRARY_PATH=$prefix/lib ldd "$library" |
            awk '$1 ~ /^lib(avcodec|avformat|avutil)\.so\./ && $3 ~ /^\// { print $3 }')
        ;;
    MINGW*|MSYS*|CYGWIN*)
        library=$prefix/bin/mpv-2.dll
        [ -f "$library" ] || die "installed libmpv runtime is missing: $library"
        if command -v llvm-objdump >/dev/null 2>&1; then
            objdump_command=llvm-objdump
        elif command -v objdump >/dev/null 2>&1; then
            objdump_command=objdump
        else
            die "llvm-objdump or objdump is required to attest Windows imports"
        fi
        linked_ffmpeg=$($objdump_command -p "$library" |
            awk 'toupper($1) == "DLL" && toupper($2) == "NAME:" &&
                 tolower($3) ~ /^av(codec|format|util)-[0-9]+\.dll$/ { print $3 }')
        ;;
    *) die "unsupported attestation platform: $(uname -s)" ;;
esac
[ -f "$library" ] || die "installed libmpv runtime is missing: $library"

printf '%s\n' "$linked_ffmpeg" | while IFS= read -r dependency; do
    [ -n "$dependency" ] || continue
    case "$(uname -s):$dependency" in
        Darwin:@rpath/*)
            resolved=$prefix/lib/${dependency#@rpath/}
            ;;
        MINGW*:*|MSYS*:*|CYGWIN*:*)
            resolved=$(find "$prefix/bin" -maxdepth 1 -type f -iname "$dependency" -print | sed -n '1p')
            ;;
        *) resolved=$dependency ;;
    esac
    case "$resolved" in
        "$prefix"/*) ;;
        *) die "libmpv links FFmpeg outside its attested prefix: $dependency" ;;
    esac
    [ -f "$resolved" ] || die "attested FFmpeg dependency is missing: $resolved"
done

for required_ffmpeg_library in avcodec avformat avutil; do
    printf '%s\n' "$linked_ffmpeg" |
        grep -Ei "(^|[/\\\\])(lib)?${required_ffmpeg_library}([.-])" >/dev/null 2>&1 ||
        die "libmpv must dynamically link $required_ffmpeg_library from the attested prefix"
done

printf 'ANKAI_LIBMPV_ATTESTATION=1\n'
printf 'MPV_GPL=false\n'
printf 'MPV_SHARED=true\n'
printf 'FFMPEG_GPL=false\n'
printf 'FFMPEG_NONFREE=false\n'
printf 'LIBMPV_SHA256=%s\n' "$(sha256_file "$library")"
printf 'BUILD_ID=%s\n' "$build_id"
printf 'CORRESPONDING_SOURCE_URL=%s\n' "$source_url"
