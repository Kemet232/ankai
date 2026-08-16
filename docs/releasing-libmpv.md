# Releasing the bundled libmpv runtime

ANKAI loads libmpv dynamically from its own application bundle. A released
desktop build must therefore contain libmpv and its non-system shared-library
dependency closure; users must not need Homebrew, a Linux package manager, or
a separate mpv installer.

This is a release-engineering control, not a legal opinion. mpv is GPL by
default. Only an intentionally built `gpl=false` shared libmpv, paired with an
FFmpeg build that has GPL and nonfree features disabled, is eligible for an
ANKAI release. Every dependency still needs its own license review.

## Trusted build inputs

Build mpv and its dependencies in an isolated, versioned prefix. At minimum,
the recorded configuration must include:

```text
mpv Meson:       -Dgpl=false -Dlibmpv=true -Ddefault_library=shared
FFmpeg configure: --disable-gpl --disable-nonfree
```

Do not use an ordinary Homebrew, distro, MSYS2, or third-party mpv binary as a
release input. Those packages are useful for development, but their feature
and licensing choices are outside ANKAI's release controls and a default mpv
build is GPL.

The installed prefix must also contain a reviewed license inventory at
`PREFIX/share/licenses` (or pass its path explicitly to the bundler). Include
the license and notice files for mpv, FFmpeg, libass, codec libraries, and
every other copied runtime dependency. Publish corresponding source for the
exact copyleft components and patches at a stable HTTPS URL. The source URL
belongs to the release, not merely to an upstream project's latest branch.

## Produce the build attestation

The trusted build job generates an attestation from the actual mpv Meson build
directory, the matching installed FFmpeg executable, and the exact libmpv
binary hash:

```sh
scripts/attest-libmpv-build.sh \
  "$prefix" \
  "$mpv_build_dir" \
  "$prefix/bin/ffmpeg" \
  "mpv-0.41.0-ankai.1-macos-arm64" \
  "https://downloads.example.invalid/ankai/sources/mpv-0.41.0-ankai.1.tar.zst" \
  > libmpv-build.env
```

The script fails unless Meson reports `gpl=false`, libmpv enabled, and a
shared-library build. It also requires FFmpeg's build configuration to record
the explicit `--disable-gpl` and `--disable-nonfree` flags and rejects either
corresponding enable flag. The attestation binds those checks to the installed
libmpv SHA-256 and verifies that its dynamic avcodec, avformat, and avutil
dependencies resolve inside that same isolated prefix. Keep the attestation
with the release provenance records.

This check cannot prove the license of every transitive codec dependency. The
reviewed source bill of materials and license inventory remain required; do
not describe the attestation as a complete license audit.

## Required application layouts

The release executable must be in the location already used by ANKAI's runtime
loader:

| Platform | Executable | Bundled libmpv |
|---|---|---|
| macOS | `ANKAI.app/Contents/MacOS/ankai` | `ANKAI.app/Contents/Frameworks/libmpv.dylib` |
| Linux | `ankai/bin/ankai` | `ankai/lib/libmpv.so.2` |
| Windows | `ankai/ankai.exe` | `ankai/mpv-2.dll` |

After building the application shell, bundle the runtime:

```sh
scripts/bundle-libmpv.sh \
  "$bundle_root" \
  "$prefix" \
  libmpv-build.env \
  "$ankai_executable"
```

The bundler performs all work in a temporary staging directory first. It then:

- verifies the attestation against the exact input library;
- discovers and copies the complete non-system dynamic-library closure;
- rewrites macOS install names to `@rpath` and adds `@loader_path`;
- sets Linux runtime-library rpaths to `$ORIGIN`;
- copies Windows DLL imports beside `ankai.exe`;
- preserves the complete supplied license inventory, attestation, and a hash
  receipt inside the release; and
- invokes the independent bundle verifier before succeeding.

Existing libmpv output or license directories cause a hard failure. Build in
a clean release staging directory instead of merging over an older artifact.

## Verification and signing order

Verification can be repeated independently:

```sh
scripts/verify-libmpv-bundle.sh "$bundle_root" "$ankai_executable"
```

It checks the executable's expected bundled lookup path, receipt hashes,
architecture compatibility on macOS, rewritten install names or rpaths,
missing dependencies, and any non-system dependency that resolves to an
absolute package-manager/system path. It also compiles a tiny loader probe and
loads the exact bundled libmpv path, resolving `mpv_client_api_version` without
initializing a player.

Run this verifier natively for every release target. macOS cannot validate PE
DLL search behavior or Linux ELF loader behavior, so a Windows and Linux CI
runner are release requirements. Cross-compilation alone is not sufficient.

On macOS the bundler ad-hoc-signs rewritten dylibs so they remain loadable for
verification. Apply the real Developer ID signature to nested libraries and
then the app, notarize, staple, and verify Gatekeeper only after bundling. On
Windows, Authenticode-sign the final DLLs and executable after bundling. Sign
the final Linux archive/package with the project's release key. Any mutation
after signing invalidates the relevant signature or receipt.

## Platform boundaries that still require release testing

- The Linux allowlist intentionally leaves glibc, the ELF loader, and graphics
  driver interfaces to the target OS. Build on the oldest supported glibc and
  test on every supported distribution; this script does not make an arbitrary
  glibc build universally portable.
- Non-system macOS frameworks are rejected instead of being copied partially.
  Add an explicit framework packaging and license policy if one becomes a
  dependency.
- Windows system-DLL classification is validated from PE imports, but the final
  closure and loader probe must run on the minimum supported Windows version.
  Compiler runtimes such as `VCRUNTIME` and `MSVCP` are not assumed present and
  must be supplied in the dependency prefix when imported.
- Hardware-decoder and graphics-driver libraries are OS interfaces and are not
  redistributed. Test software-decoding fallback and each supported hardware
  path on physical release hardware.
