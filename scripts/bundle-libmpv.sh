#!/bin/sh
set -eu

# Copies an LGPL-configured libmpv distribution into an ANKAI release bundle.
# Usage: scripts/bundle-libmpv.sh <app-or-release-dir> <libmpv-prefix>
# The prefix must be produced from mpv with `meson setup build -Dgpl=false`.

destination=${1:?destination app/release directory is required}
prefix=${2:?LGPL libmpv prefix is required}

case "$(uname -s)" in
  Darwin)
    framework_dir="$destination/Contents/Frameworks"
    mkdir -p "$framework_dir"
    cp "$prefix/lib/libmpv.dylib" "$framework_dir/libmpv.dylib"
    ;;
  Linux)
    library_dir="$destination/lib"
    mkdir -p "$library_dir"
    cp "$prefix/lib/libmpv.so.2" "$library_dir/libmpv.so.2"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    cp "$prefix/bin/mpv-2.dll" "$destination/mpv-2.dll"
    ;;
  *)
    echo "unsupported packaging platform" >&2
    exit 1
    ;;
esac

license_file="$prefix/share/licenses/mpv/Copyright"
if [ -f "$license_file" ]; then
  mkdir -p "$destination/licenses"
  cp "$license_file" "$destination/licenses/libmpv-Copyright"
else
  echo "libmpv license file missing; refusing to produce a redistributable bundle" >&2
  exit 1
fi
