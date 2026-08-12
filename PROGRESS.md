# ANKAI — Progress / Resume Point

> **If you are picking this up cold (new session, ran out of context, etc.):**
> Read this file top to bottom, then skim `docs/adr/*.md` for decisions already locked in.
> That's enough to resume without re-reading the full product spec.

Last updated: 2026-08-13

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

**Phase 0 — Architecture.** Nothing user-facing built yet. In progress:

| Track | Status | Owner artifact |
|---|---|---|
| Repo scaffold | done | this repo |
| ADR process | done | `docs/adr/0001-*.md` |
| Native UI stack decision | **Accepted: Slint** (fallback: Qt/QML via cxx-qt) | `docs/adr/0002-native-ui-stack.md` |
| P2P networking stack decision | **Accepted: iroh (QUIC) + WebRTC** | `docs/adr/0003-p2p-networking-stack.md` |
| E2EE stack decision | researching | `docs/adr/0004-e2ee-stack.md` |
| Emulation integration | **Accepted: shell out to RetroArch/standalone emulators, no bundling** | `docs/adr/0005-emulation-integration.md` |
| Media playback engine | **Accepted: libmpv + wasmtime-sandboxed providers** | `docs/adr/0006-media-playback-engine.md` |
| Open-source model (what's open vs. closed) | not started, blocks LICENSE file | new ADR, see task list |
| Threat model | done (v0, will grow as ADRs land) | `docs/threat-model.md` |
| Design tokens (Liquid Y2K) | done (v0) | `docs/design/tokens.md` |
| Rust workspace skeleton | `core` crate scaffolded + builds; `client` crate not started (now unblocked — Slint picked) | `core/`, `client/` |
| CI | not started | `.github/workflows/` |

## Immediate next steps (in order)

1. Accept ADR-0004 (E2EE stack) once its research agent finishes — still in flight.
2. Write the open-source-model ADR (what's open vs. closed/hosted, spec section 42)
   — factor in ADR-0004's licensing findings (e.g. libsignal is AGPL) before
   picking; this blocks adding a LICENSE file to the now-public repo.
3. Start the `client` crate (Slint, per ADR-0002) in the Cargo workspace: window,
   nav skeleton. Run the two spikes ADR-0002 flags (glass/blur rendering,
   accessibility validation) early, before deep UI investment.
4. Stand up CI (`fmt`, `clippy`, `test`) in `.github/workflows/ci.yml` once the
   `client` crate exists (a Rust CI for a workspace with no real code yet is not
   useful).
5. Begin Phase 1 (native shell: window, nav, theme foundation, SQLite, settings)
   per the phase list — see any ADR or ask the user for the full phase breakdown
   if it's not already summarized in this file by then.

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

## Open questions for the human

- Repo is public under the `Kemet232` GitHub account (no LICENSE file yet,
  so it's all-rights-reserved by default in the meantime).
- Open-source model (spec section 42: what's open — protocol, crypto, client
  core, SDKs — vs. closed/hosted — marketplace backend, abuse infra,
  recommendations) hasn't been decided. Needs its own ADR before adding a
  LICENSE file or accepting outside contributions.

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
