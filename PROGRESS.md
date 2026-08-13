# ANKAI — Progress / Resume Point

> **If you are picking this up cold (new session, ran out of context, etc.):**
> Read this file top to bottom, then skim `docs/adr/*.md` for decisions already locked in.
> That's enough to resume without re-reading the full product spec.

Last updated: 2026-08-13 (session 2, cont'd)

## What ANKAI is

Native (no webview) social platform for anime/gaming communities: profiles, Top 8,
E2EE messaging, communities/forums, watch-party "Hangouts", P2P-first networking,
emulation frontend, creator marketplace (90/10 split). Full spec lives in the
original directive — not duplicated here; ask the user if you need the verbatim text,
or infer scope from the ADRs and phase list below.

Philosophy: **thin cloud, fat client** — central servers do identity/discovery/
moderation/marketplace metadata only; bandwidth-heavy traffic (DMs, voice, watch
parties, theme assets) goes peer-to-peer wherever safe.

## Repo

- Local: `~/Projects/ankai`
- GitHub: https://github.com/Kemet232/ankai (public)

## Current phase

**Phase 0 — Architecture: done.** **Phase 1 — Native shell: started.**

| Track | Status | Owner artifact |
|---|---|---|
| Repo scaffold | done | this repo |
| ADR process | done | `docs/adr/0001-*.md` |
| Native UI stack decision | **Accepted: Slint** (fallback: Qt/QML via cxx-qt) | `docs/adr/0002-native-ui-stack.md` |
| P2P networking stack decision | **Accepted: iroh (QUIC) + WebRTC** | `docs/adr/0003-p2p-networking-stack.md` |
| E2EE stack decision | **Accepted: MLS via OpenMLS (unified 1:1+group)** | `docs/adr/0004-e2ee-stack.md` |
| Emulation integration | **Accepted: shell out to RetroArch/standalone emulators, no bundling** | `docs/adr/0005-emulation-integration.md` |
| Media playback engine | **Accepted: libmpv + wasmtime-sandboxed providers** | `docs/adr/0006-media-playback-engine.md` |
| Open-source model (what's open vs. closed) | **Accepted: Apache-2.0** for core/client/protocol/SDKs; marketplace backend, abuse infra, recommendation internals stay closed | `docs/adr/0007-open-source-model.md`, `LICENSE` |
| Threat model | done (v0, will grow as ADRs land) | `docs/threat-model.md` |
| Design tokens (Liquid Y2K) | done (v0) | `docs/design/tokens.md` |
| Rust workspace skeleton | `core` + `client` crates scaffolded, both build clean (fmt/clippy pass) | `core/`, `client/` |
| CI | done — fmt/clippy/test + cargo-deny, all generic across the workspace | `.github/workflows/ci.yml`, `deny.toml` |
| ADR-0002 spike 1 (glass/blur rendering) | prototype built and visually confirmed on dev hardware; low-end-machine frame-time profiling still open | `docs/adr/0002-native-ui-stack.md` ("Spike 1 results") |
| Theme tokens module | done — `client/ui/theme.slint` (Slint `global Theme`), all of tokens.md's colors/spacing/motion mapped; app.slint/module-card.slint refactored off raw hex | `client/ui/theme.slint` |
| Window + nav skeleton | done — real `client` binary default output now; deliberately plain (no glass/blur) per ADR-0002's gate | `client/ui/app.slint`, `client/src/main.rs` |
| Local encrypted DB (SQLite/SQLCipher) | done — `Db::open`/`open_in_memory`, migrations scaffold, wrong-key-fails test | `core/src/db.rs` |

The old glass/blur + drag-reorder spike still exists (now themed via
`Theme.*`) at `client/ui/spike-glass-blur.slint`, reachable only via
`cargo run -p client -- --spike` (or `ANKAI_SPIKE_DEBUG=1`) — it is not
the default UI anymore.

## Immediate next steps (in order)

All 5 research ADRs (0002-0006) are Accepted, plus ADR-0007 (open-source
model: Apache-2.0, LICENSE added). `ankai-core`'s `identity` module has
MLS/OpenMLS-shaped types (real `openmls` types where safe, honest
placeholders elsewhere — see `core/src/identity.rs` doc comments), builds
clean.

Session 2 ran three Phase-1 tracks in parallel (isolated git worktrees),
reviewed and merged all three onto `main`: theme tokens module, window+nav
skeleton, and the SQLite/SQLCipher `db` module in `core`. The nav-skeleton
and theme-tokens branches both touched `client/ui/app.slint` (one renamed
its content to `spike-glass-blur.slint`, the other refactored it in place)
so merging required manually resolving that conflict — done by taking the
nav skeleton as the new `app.slint` and reapplying the `Theme.*` refactor to
`spike-glass-blur.slint` by hand. Post-merge, full workspace
build/test/clippy/fmt all verified clean, and the running app was visually
screenshotted to confirm no regression in either the nav shell or the
(now-legacy) spike.

**Note for future sessions:** during this work, an agent testing the nav
skeleton attempted synthetic OS-level mouse clicks (no accessibility
permissions were available for cleaner automation) and one click missed the
app window and landed on the human's own browser tab. Avoid this class of
action — prefer code-level verification of simple state-driven UI logic
(e.g. confirming a `TouchArea.clicked` handler and its binding by reading
the `.slint` source) over synthetic input on the user's live desktop. A
project-level `run`/screenshot skill for this repo, if one gets built,
should account for this.

Remaining steps:

1. **ADR-0002 spike 1 is not fully closed** — frame-time profiling on a real
   low-end reference machine (old dual-core, integrated GPU, 4GB RAM, real
   or virtualized — pick and document one) hasn't been done, only a
   qualitative check on capable dev hardware. Do that before sinking more
   engineering into the glass/blur visual system, per the ADR's own gate.
2. Keep an eye on ADR-0002's spike-2 (accessibility validation with real
   screen readers) — that one requires an actual human, it can't be
   delegated to an agent.
3. Continue Phase 1: settings screen/persistence (wire the nav skeleton's
   "Settings" pane to something real, backed by `core`'s new `db` module),
   SQLite key management (currently `Db::open` takes a passphrase from the
   caller — OS secure-storage-backed key derivation is still unimplemented,
   see `core/src/db.rs` module doc comment), and wiring `Theme` into the nav
   skeleton (`app.slint` is currently plain/flat on purpose — swap in real
   tokens once appropriate, it was deliberately left out of both agents'
   scope so they wouldn't collide).

Remember: the E2EE integration itself (not the underlying libraries) requires
an independent professional security audit before shipping to real users —
see ADR-0004's "Required before shipping" gate. Don't let scaffolding progress
create false confidence that this gate has been cleared.

## Decisions locked so far

- **UI toolkit**: Slint (Rust API, royalty-free license), Skia backend for full
  effects / software backend for Potato mode. Fallback if the blur/glass spike
  fails: Qt/QML via cxx-qt. See `docs/adr/0002`.
- **P2P/networking**: iroh (QUIC-native) for messaging/signalling/files/netplay/
  spectating; WebRTC (`webrtc-rs`/`str0m`) for voice/video/screenshare. See
  `docs/adr/0003`.
- **Emulation**: never bundle/link emulator code — shell out to separately
  installed RetroArch (libretro cores) and standalone per-system emulators as
  external processes; ANKAI provides the social/matchmaking/netplay-signalling
  layer only. See `docs/adr/0005`.
- **Media playback**: embed libmpv (dynamically linked LGPL) for all video
  playback/hardware decode/subtitles; third-party "provider" plugins run as
  sandboxed WASM components (wasmtime, WASI 0.2) with explicit per-provider
  capability grants. See `docs/adr/0006`.
- **E2EE**: unify on MLS (RFC 9420) via OpenMLS for both 1:1 DMs (as 2-member
  groups) and group/community channels — not a separate Signal-Protocol-style
  double ratchet for 1:1. Voice/video E2EE via SFrame keyed from MLS's
  `exporter_secret`; local DB encrypted via SQLCipher; recovery via a client-
  side-encrypted recovery-key backup (server never sees plaintext or keys).
  `libsignal`/RingRTC explicitly rejected (AGPL, conflicts with closed
  marketplace backend); vodozemac rejected for now (disputed unpatched
  high-severity disclosure, Feb 2026, not a license issue). Independent
  security audit of ANKAI's own integration work is a hard gate before
  shipping to real users. See `docs/adr/0004`.

## Open questions for the human

- Repo is public under the `Kemet232` GitHub account, now Apache-2.0
  licensed (`core`, `client`, protocol, SDKs) per `docs/adr/0007`. Marketplace
  backend, abuse infra, and recommendation-engine internals stay closed —
  say if you disagree with that split.
- Whether/when to formally open the repo to outside PRs is still undecided
  (public+licensed just means readable/usable, not "accepting contributions"
  — see README).

## Session hygiene

Commit + push after every meaningful chunk of work, and keep this file's
"Current phase" table and "Immediate next steps" current as you go — don't
batch updates for the end. If a session is running long (context filling up,
lots of accumulated tool output), checkpoint proactively: finish or cleanly
abandon the in-flight edit, update this file, commit/push, and stop rather
than risk getting cut off mid-change with uncommitted state.

## How to resume seamlessly (read this if you're a future/fresh session)

1. `cd ~/Projects/ankai && git log --oneline -20` — see what actually landed.
2. Read this file (`PROGRESS.md`) — it's kept current after every work session.
3. Read `docs/adr/` — every non-trivial technical choice is recorded there with
   rationale, not just the conclusion. Don't re-litigate an Accepted ADR without
   new information.
4. Check the "Immediate next steps" list above and continue from the first
   undone item.
5. If the user just says "continue ANKAI" with no other context, that's enough —
   do not re-ask them to re-explain the product.
