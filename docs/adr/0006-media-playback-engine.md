# 6. Media playback engine

Status: Accepted
Date: 2026-08-13

## Context

ANKAI needs an anime-viewing feature covering local files (a user's own
library), legally authorized remote sources, and a pluggable "provider"
model so the community/third parties can add sources without ANKAI
shipping (or being legally responsible for) any specific streaming
integration. Requirements, per the product spec's licensing-review and
capability-prompt norms:

- Hardware-accelerated video decode across macOS, Windows, and Linux.
- Subtitle rendering (styled ASS/SSA, the anime-fansub-community standard,
  not just basic SRT).
- A plugin architecture where **user/community/third-party provider code
  is untrusted** and must be sandboxed — it should only get network access
  to the specific domain(s) it declares, and filesystem access should be
  similarly capability-gated, matching the product's "Provider requests:
  internet access to domain X" permission-prompt model.
- No linking/licensing choice that forces ANKAI's client to become
  GPL-licensed.

### libmpv

mpv (and its embeddable `libmpv`) is the de facto standard for
high-quality, hardware-accelerated, subtitle-correct video playback —
it's what Jellyfin Media Player, Celluloid, IINA, and many others already
embed rather than reimplementing a player on raw FFmpeg.

**Licensing is nuanced and load-bearing for ANKAI:**
- mpv's core is licensed **GPLv2-or-later** by default.
- mpv can be built in an **LGPLv2.1+ configuration** by passing
  `-Dgpl=false` to its Meson build (historically `--enable-lgpl`). This
  configuration excludes a specific set of GPL-only components — notably
  the X11 windowing backend, V4L2 TV input, and DVD (libdvdnav) support —
  none of which ANKAI needs (we're not building an X11-only Linux window
  manager integration or DVD playback).
- The **`libmpv` client API header (`client.h`) is ISC-licensed**
  independent of the GPL/LGPL build flag, specifically so that the
  *interface* is unencumbered; mpv's own docs note it's "up to lawyers"
  whether calling that API from a separately-distributed application pulls
  the caller under GPL/LGPL — the safe, conventional reading (matching how
  IINA, Celluloid, and Jellyfin Media Player already ship) is: **dynamic
  linking against an LGPL-built libmpv, as a separate shared library the
  user's OS loads at runtime, does not require ANKAI's own source to be
  GPL/LGPL**, the same way any app dynamically linking `glibc` or `Qt`
  (LGPL) stays proprietary. Static linking or vendoring mpv source
  directly into the ANKAI binary would be a materially different,
  riskier posture and should be avoided.
- **Action for ANKAI**: build/obtain libmpv with `-Dgpl=false`
  specifically, dynamically link it (not static/vendored), and record it
  in the licensing manifest as LGPLv2.1+ (client API surface ISC). This is
  the same approach FFmpeg-adjacent commercial-friendly players already
  use.

### Rust bindings

- `mpv-rs` (crates.io `mpv`, by Cobrand): safe libmpv bindings, MIT/Apache-2.0
  dual-licensed crate. Actively referenced but should be checked for
  current maintenance activity before depending on it long-term — the
  ecosystem also has `libmpv2-sys` and `libmpv-sys` (both LGPL-2.1
  themselves, being thin FFI layers over the LGPL library) as lower-level
  alternatives if `mpv-rs`'s safe wrapper is stale.
- **Recommendation**: use `libmpv2-sys`/an actively maintained low-level
  FFI crate and write a thin safe wrapper in ANKAI's own `core`/`client`
  crate scoped to exactly what ANKAI needs (load file, seek, set
  subtitle track, query hw-decode status, property observation for
  presence/"currently watching" state) rather than depending on a
  potentially-unmaintained full wrapper crate. This keeps the FFI surface
  small and auditable, which matters given libmpv's C API is large.

### Hardware-accelerated decode

libmpv already abstracts this — **ANKAI should not talk to platform
decode APIs directly.** mpv/FFmpeg's hwaccel layer picks the right backend
per OS:
- **macOS**: VideoToolbox (`--hwdec=videotoolbox`).
- **Windows**: Direct3D 11/12 video acceleration (DXVA2/D3D11VA/D3D12VA)
  via Media Foundation.
- **Linux**: VA-API (Intel/AMD) or NVDEC, via `--hwdec=vaapi`/`nvdec`.

ANKAI's job is just to set `hwdec=auto-safe` (or explicit per-platform
default) and expose a settings toggle; mpv handles backend selection,
fallback to software decode, and zero-copy render-path integration with
whatever windowing surface ANKAI's UI toolkit (ADR-0002) uses.

### Subtitles: libass

mpv already embeds/uses **libass** (ISC-licensed) for ASS/SSA subtitle
rendering — full styling, karaoke effects, positioning, the format the
fansub/anime community standardizes on, distinct from and more capable
than basic SRT. No separate integration needed: enabling subtitle tracks
through libmpv gets libass rendering for free. ISC is permissive and adds
no licensing complication.

### Provider architecture: sandboxing untrusted plugin code

The core design question is how a "provider" (local library scanner,
an authorized-streaming-source connector, or a third-party/community
plugin) supplies playable media URLs/streams to the player while running
as **untrusted code** ANKAI didn't write and can't fully vet.

**Decision driver**: WebAssembly sandboxing is the right primitive here,
not OS-process sandboxing or a scripting-language sandbox (Lua/JS engines
have historically leaky sandboxes; WASM's isolation is structural, not
policy-based).

- **wasmtime** (Bytecode Alliance) is the reference-grade runtime; as of
  2026 it fully implements **WASI Preview 2 / WASI 0.2** (stabilized
  January 2024), including the **Component Model**, which lets a plugin's
  interface be described in a typed IDL (WIT) rather than raw bytes over
  shared memory. Wasmtime is also tracking WASIp3 previews, but ANKAI
  should target the stable 0.2 baseline, not bleeding-edge previews.
- **The security model is exactly what ANKAI's product spec wants**: WASI
  components start with **zero ambient authority** — no filesystem, no
  network, no clock, no env vars, no syscalls — and every capability
  (e.g. "open a TCP connection to `api.provider.example`", "read this one
  directory") must be an explicit typed import that the **host (ANKAI)**
  grants at instantiation time. This maps directly onto the "Provider
  requests: internet access to domain X" permission-prompt UX: ANKAI's
  host code is the only thing that can grant a network-capable handle,
  and it can scope that handle to a single domain (or a small allowlist)
  rather than general sockets — the plugin literally cannot reach any
  other host even if its code tries, because it never receives a general
  "open any socket" capability at all.
- **Extism** (built on wasmtime) is a purpose-built framework for exactly
  this shape of problem — "load untrusted WASM plugins into a host app,
  move data across the boundary safely" — with PDKs for writing plugins
  in Rust, Go, JS, AssemblyScript, C, Zig, etc., so third-party/community
  provider authors aren't forced to write Rust. Worth evaluating directly
  against a hand-rolled wasmtime embedding; Extism trades a small amount
  of control for a lot of boilerplate (host-guest memory marshalling,
  function tables) that ANKAI would otherwise have to build itself.

**Recommendation**: build the provider sandbox on **wasmtime directly**
(or via Extism if its abstraction proves sufficient after a spike),
targeting WASI 0.2/Component Model, with:
1. A ANKAI-defined WIT interface for what a "provider" can export
   (`search(query) -> [MediaItem]`, `get_stream(id) -> StreamHandle`,
   `get_subtitle_tracks(id) -> [...]`) and import (a capability-scoped
   `fetch(url)` that ANKAI's host validates against the domain(s) the
   provider declared and the user approved, a capability-scoped local
   file reader for the local-library provider case only).
2. Every provider ships a manifest declaring requested capabilities
   (domains for network access, directories for filesystem access) —
   surfaced to the user as an install-time/first-run permission prompt,
   revocable later, matching the product's stated permission model.
3. Resource limits (fuel/epoch-based CPU limits, memory limits) enforced
   by wasmtime per-instance so a misbehaving or hostile provider can't
   hang or OOM the client.
4. First-party providers (local library, and any officially-negotiated
   streaming-partner integration) can be *implemented* the same way as
   third-party ones (dogfooding the sandbox/API), even though they ship
   in-box and are trusted by ANKAI — this keeps one code path instead of
   a trusted/untrusted fork, and means a first-party provider that later
   gets compromised or misbehaves is contained exactly like a third-party
   one would be.

### Legal posture for remote/streaming providers specifically

This is a *policy* decision as much as a technical one, and needs to be
explicit given ANKAI ships a plugin marketplace surface:

- There is no evidence of an open, self-serve "developer API" from major
  licensed anime streaming services (e.g. Crunchyroll) for third-party
  playback apps — access of that kind is typically only available via
  direct business/licensing partnership, not a public API key signup.
  **ANKAI must not ship, and must not allow the marketplace to list, any
  provider that scrapes or reverse-engineers a paid streaming service's
  private API** — that's a ToS-violating, legally hostile integration
  pattern regardless of sandboxing, and the sandbox does not launder that
  risk away.
- **Self-hosted/personally-owned media servers are unambiguously fine**:
  a "Jellyfin provider" or "Plex provider" is just an HTTP client against
  a server the *user* stood up over *their own* media (Jellyfin itself is
  GPLv2 server-side, but ANKAI would only ever be consuming its open REST
  API over the network — no linking, no licensing entanglement, same
  "separate program over a network/process boundary" logic as ADR-0005's
  emulator posture).
  - Local-library and self-hosted-server providers should be the
    first-party, in-box default; anything requiring a live paid-service
    integration should require ANKAI to have an actual commercial/legal
    agreement with that provider before it ships as first-party, and
    third-party/community providers of that kind should go through
    licensing review before marketplace listing (flag: this is exactly
    the kind of provider submission that needs a human legal check, not
    just a capability-permission technical check).

## Decision

**ANKAI embeds libmpv (built LGPLv2.1+, dynamically linked, never
statically vendored) as the playback engine for all video, covering local
files and any provider-supplied stream URL uniformly.** Hardware decode
and subtitle rendering (libass) come for free through libmpv/FFmpeg's
existing cross-platform abstraction — ANKAI does not touch
VideoToolbox/Media Foundation/VA-API directly. A thin, purpose-built Rust
FFI wrapper (built on `libmpv2-sys` or equivalent low-level bindings, not
a full third-party wrapper crate) exposes exactly the surface ANKAI's
client needs.

**"Providers" (local library, self-hosted servers, authorized
partnerships, community/third-party plugins) are WebAssembly components
run under wasmtime, targeting WASI 0.2/Component Model, with zero ambient
authority and per-provider, user-visible, revocable capability grants for
network domains and filesystem paths.** All providers — including
first-party ones — go through the same sandboxed interface; only the
ones requiring a live third-party paid-service integration require a
separate legal/licensing review gate before being offered, in addition to
the technical capability-permission gate every provider goes through.

Release artifacts follow the fail-closed build-attestation, dependency-closure,
loader verification, and native-per-platform signing process in
[`../releasing-libmpv.md`](../releasing-libmpv.md). A development machine's
package-manager libmpv is never a valid release input.

### Licensing manifest entries required

| Component | License | How ANKAI uses it | Notes |
|---|---|---|---|
| `libmpv` (built `-Dgpl=false`) | LGPLv2.1+ (client API: ISC) | Dynamically linked shared library | Must confirm build excludes GPL-only components (X11, V4L2, DVD); do not static-link |
| `libass` | ISC | Transitively via libmpv, no direct dependency | Permissive, no action needed |
| FFmpeg (mpv's decode backend) | LGPL or GPL depending on build config | Transitively via libmpv | Must match libmpv's LGPL build — verify FFmpeg build used is also LGPL-configured, not GPL-configured, or the combination reintroduces GPL |
| `mpv-rs` / `libmpv2-sys` (whichever chosen) | MIT/Apache-2.0 or LGPL-2.1 | Rust FFI layer | Confirm current maintenance status before pinning |
| wasmtime | Apache-2.0 with LLVM exception | Provider sandbox runtime | Permissive, no conflict |
| Extism (if adopted over raw wasmtime) | BSD-3-Clause | Provider plugin framework | Permissive, no conflict |
| Any first-party streaming-partner provider | N/A (business agreement, not OSS license) | — | Requires actual legal/commercial agreement, tracked separately from OSS license manifest |
| Third-party/community providers submitted to marketplace | Varies (provider author's own license/ToS for their backend) | — | Route through licensing + legal review before marketplace listing, not just automated capability-manifest check |

## Consequences

**Easier:**
- One playback engine (libmpv) handles every codec/container/subtitle
  format ANKAI is likely to encounter, with hardware decode "for free" on
  all three target OSes — avoids reinventing a decode pipeline or
  chasing platform-specific APIs directly.
- The provider sandbox gives ANKAI a real answer to "how do we let the
  community extend this without becoming liable for what they build,"
  which is required before any plugin marketplace can exist at all —
  WASI's zero-ambient-authority model maps almost one-to-one onto the
  product's stated permission-prompt UX, so the technical design and the
  product design reinforce each other instead of fighting.
- Dynamic-linking LGPL libmpv keeps ANKAI's client closed-source without
  requiring a commercial mpv license (mpv has no such thing to buy in the
  first place — LGPL build is the standard path every non-GPL commercial
  mpv-embedding app already uses).

**Harder / traded away:**
- ANKAI must build and maintain its own Rust FFI wrapper around libmpv's
  C API rather than depending on a possibly-thin third-party crate —
  more surface area to test and keep memory-safe at the FFI boundary.
- The LGPL build of libmpv drops some GPL-only features (X11-native
  windowing, DVD playback, V4L2 TV capture); none are currently needed,
  but any future feature request that needs one of those specifically
  would force a licensing re-evaluation, not just a code change.
- The WASM provider sandbox is real engineering investment: a WIT
  interface design, a capability-grant UI, resource-limit tuning, and a
  plugin SDK/PDK story for third-party authors — this is not a thin
  wrapper, it's a subsystem in its own right, and its scope should be
  budgeted accordingly rather than treated as an afterthought bolted onto
  the player.
- Legal review becomes an ongoing product process, not a one-time ADR
  decision: every marketplace-listed provider that talks to a live
  paid/licensed service needs a human legal check that the sandbox cannot
  automate away, and ANKAI needs to actively police the marketplace
  against ToS-violating scraper-style providers being submitted.
- No official public developer API surfaced for major licensed anime
  streaming platforms means ANKAI's own "authorized remote source"
  offering is realistically limited, at least initially, to self-hosted
  servers (Jellyfin/Plex-style) and whatever direct partnerships ANKAI
  separately negotiates — this is a business-development dependency, not
  something engineering can solve alone.

Sources consulted: mpv-player/mpv GitHub (Copyright file, LGPL relicensing
issue #2033), mpv build docs (`-Dgpl=false`), libass GitHub/FSF Free
Software Directory, crates.io (`mpv`, `libmpv-sys`, `libmpv2-sys`),
Bytecode Alliance / wasmtime docs and security page, Extism
docs/GitHub, Jellyfin docs/GitHub, general-web reporting on Crunchyroll's
2026 licensing/partnership model (no public self-serve developer API
found), GNU.org LGPL background.
