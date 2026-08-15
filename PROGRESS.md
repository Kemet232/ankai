# ANKAI — Progress / Resume Point

> **If you are picking this up cold (new session, ran out of context, etc.):**
> Read this file top to bottom, then skim `docs/adr/*.md` for decisions already locked in.
> That's enough to resume without re-reading the full product spec.

Last updated: 2026-08-15 (session 11)

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
| ADR-0002 spike 1 (glass/blur rendering) | **closed** — validated on dev hardware and on an old dual-core/integrated-GPU/4GB-RAM machine (user-confirmed) | `docs/adr/0002-native-ui-stack.md` ("Spike 1 results") |
| Theme tokens module | done — `client/ui/theme.slint` (Slint `global Theme`), all of tokens.md's colors/spacing/motion mapped; app.slint/module-card.slint refactored off raw hex | `client/ui/theme.slint` |
| Window + nav skeleton | done — real `client` binary default output now; deliberately plain (no glass/blur) per ADR-0002's gate | `client/ui/app.slint`, `client/src/main.rs` |
| Local encrypted DB (SQLite/SQLCipher) | done — `Db::open`/`open_in_memory`, migrations scaffold, wrong-key-fails test, `settings` key-value table (migration 2) with get/set | `core/src/db.rs` |
| DB key management (OS secure storage) | done — `keychain::device_db_passphrase()` generates a passphrase on first run and stores it via `keyring` (macOS Keychain/Windows Credential Manager/Linux Secret Service); `client`'s `main.rs` now actually opens the real encrypted DB with it instead of never touching `core::db` | `core/src/keychain.rs`, `client/src/main.rs` |
| Settings screen wired to real persistence | done — nav skeleton's Settings pane has a working "display name" field backed by the `settings` table (load on startup, save via button/Enter) | `client/ui/app.slint`, `client/src/main.rs` |
| Theme tokens wired into nav skeleton | done — `app.slint`'s hardcoded hex literals replaced with `Theme.*` tokens (see ADR-0002 note below); still deliberately flat, no glass/blur | `client/ui/app.slint` |
| OpenMLS storage backed by SQLCipher DB | done — `AnkaiMlsProvider` wraps `openmls_rust_crypto`'s `MemoryStorage`, persisting its whole contents as one opaque blob in a new `mls_storage` table (migration 3); whole-blob durability granularity, not per-field — see the module doc comment for the tradeoff and when to revisit it | `core/src/mls_provider.rs` |
| Basic P2P scaffolding (iroh) | done — `P2pNode` wraps a bound iroh `Endpoint` (bind/addr/send/accept_and_echo_once/close), `presets::Minimal` so no discovery/relay service is contacted; deliberately no discovery, privacy-mode gating, relay tiers, or blobs/gossip — see the module doc comment | `core/src/p2p.rs` |
| First-run local identity | done — `identity::load_or_create_device` generates a real ED25519 signature keypair + random opaque account/device ids on first run, reloads the same identity on every run after; `AccountId` is still an honest placeholder (not a real account-root identity key — see the module doc comment); Profile pane displays both ids | `core/src/identity.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| Real MLS KeyPackage generation | done — `identity::create_key_package` builds a real `KeyPackage` via `KeyPackage::builder().build()`, using the device's stored signature key as signer and a new explicit `CIPHERSUITE` constant; stops at "build and store locally," core-only (no directory service to publish to, so not wired into `client` — asked and confirmed) | `core/src/identity.rs` |
| Local Communities feature | done — `core::communities` (create/list/get against a new `communities` table, migration 4); Communities nav pane wired end-to-end (name field + Create button, backed by a real callback + list model); explicitly local-only, no membership/networking yet — see the module doc comment | `core/src/communities.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| Shared hex/random-id helper | done — `core::util` consolidates what had become three near-identical "N random bytes → hex string" implementations (`keychain`, `identity`, and communities' id generation) | `core/src/util.rs` |
| Messages P2P stub | done — `core::messaging` (send/receive plaintext over `core::p2p`, JSON-encoded `EndpointAddr` for out-of-band address sharing) plus a real Messages nav pane (own address, paste-a-peer field, send, in-memory log); `p2p.rs` gained a persistent `accept_loop` alongside its original one-shot `accept_and_echo_once`; deliberately no discovery, no MLS/E2EE, no DB persistence yet — see `core/src/messaging.rs`'s module doc comment | `core/src/messaging.rs`, `core/src/p2p.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| ADR-0003 spike 1 tool (NAT-traversal probe) | done — new `tools/nat-probe` binary crate (uses iroh's `N0` preset, unlike `core::p2p`'s deliberately-`Minimal` production scaffolding) reports direct-vs-relay path selection and RTT via iroh's own `Connection::paths()` introspection; verified with a real localhost smoke test (both sides agreed: direct path selected, relay path present-but-unused). Spike itself is **not closed** — only the tool exists; running it across a real diverse-network cohort is still unstarted human/future work, same bar as ADR-0002's spikes | `tools/nat-probe/`, `docs/adr/0003-p2p-networking-stack.md` ("Spike 1 status") |
| Directory-service client interface (`core::directory`) | done — `DirectoryService` trait (publish/lookup a device's `KeyPackage`s, publish/lookup a device's current `EndpointAddr`) plus one in-memory implementation for tests; explicitly not wired into `client` and not a real server — no server technology has been chosen (that's flagged as a genuinely ADR-shaped decision for the human, not made here) | `core/src/directory.rs` |
| MLS-encrypted + persisted messaging | done — `core::messaging` now does real OpenMLS 2-member-group encryption per peer (`PeerInvite` bundles an `EndpointAddr` + a fresh `KeyPackage` into one paste-able blob, since there's still no directory service to publish/fetch through); messages persist as plaintext (post-decryption) in new `conversations`/`messages` tables (migration 6 — see note below on the migration-5 collision with Hangouts); still no discovery service, no multi-conversation UI, no delivery guarantees | `core/src/messaging.rs`, `core/src/db.rs` |
| Hangouts pane | done — `core::hangouts` (create/list/get against a new `hangouts` table, migration 5), mirroring Communities' shape exactly; local-only hosting metadata, no real-time sync/media/participants yet | `core/src/hangouts.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| ADR-0003 spike 2 tool (WebRTC audio/E2EE prototype) | done — new `tools/webrtc-audio-probe` binary crate: a real two-process `webrtc-rs` v0.20 voice-call prototype (manual SDP offer/answer exchange, synthetic-tone-over-real-Opus/RTP, DTLS-SRTP confirmed active via `get_stats()`). Verified end-to-end: both sides reach `Connected`, audio verification passes. Spike itself is **not closed** — subjective audio quality on real hardware, frame-level E2E through an SFU, and multi-OS validation are all still unstarted human/future work | `tools/webrtc-audio-probe/`, `docs/adr/0003-p2p-networking-stack.md` ("Spike 2 status") |
| ADR-0008: identity/discovery service + reference backend | done — `docs/adr/0008-identity-discovery-service.md` (Status: **Proposed**, researched: axum vs. tonic, SQLite vs. Postgres, ED25519-request-signing auth reusing `identity.rs`'s existing device key, TOFU pubkey pinning); `server/directory/` is a real reference implementation (axum + SQLite, KeyPackage-consumed-on-lookup + 30-day expiry, EndpointAddr 10-minute TTL) plus `HttpDirectoryClient`, a second real `DirectoryService` impl proven against it via real signed-request integration tests (including rejection of unsigned/badly-signed/impersonating publishes). Deliberately not wired into `client` — still `Proposed`, real gaps (TOFU bootstrapping, unauthenticated EndpointAddr lookups vs. the threat model, SQLite-single-process) documented as pre-`Accepted` blockers | `docs/adr/0008-identity-discovery-service.md`, `server/directory/` |
| Directory server wired into `client` (opt-in) | done — if `ANKAI_DIRECTORY_URL` is set, the client publishes its own `KeyPackage`/`EndpointAddr` to the real directory server on startup and offers a "look up by Device ID" field in Messages, replacing the need to paste a whole invite blob; unset (the default) behaves exactly as before, manual-paste-only. Real end-to-end proof: a test spins up the actual compiled `ankai-directory-server` binary as a separate OS process and confirms a full cross-process lookup + MLS-encrypted message round-trip through it | `client/src/directory.rs`, `client/src/main.rs`, `server/directory/tests/external_process_e2e.rs` |
| Community discussion posts (`core::forum_posts`) | done — flat, chronological, append-only text posts inside a community (the "forum" half of "communities/forums"); no replies, editing, or moderation yet. Wired into the Communities pane: selecting a community loads its posts, a real field + button posts new ones | `core/src/forum_posts.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| Profile customization: "Top 8 Communities" (`core::top8`) | done — MySpace-style featured-items picker, honestly scoped to communities (not "friends," since there's no real contacts model yet); pick/reorder/remove up to 8, reuses the existing `settings` table rather than a new migration | `core/src/top8.rs`, `client/ui/app.slint`, `client/src/main.rs` |

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

**Note for future sessions:** during session 2, an agent testing the nav
skeleton attempted synthetic OS-level mouse clicks (no accessibility
permissions were available for cleaner automation) and one click missed the
app window and landed on the human's own browser tab. Avoid this class of
action — prefer code-level verification of simple state-driven UI logic
(e.g. confirming a `TouchArea.clicked` handler and its binding by reading
the `.slint` source) over synthetic input on the user's live desktop.
Screenshotting the running app (no clicks) to visually confirm rendering is
fine and was done again in session 3. A project-level `run`/screenshot skill
for this repo, if one gets built, should account for this.

**ADR-0002 spike 1 is now closed** — the human validated frame time and
Potato-mode headroom recovery on an old dual-core/integrated-GPU/4GB-RAM
machine, matching the reference profile the ADR called for (confirmed
2026-08-13; see the ADR's "Spike 1 results" section — exact numeric
frame-time/headroom figures weren't separately logged, only the pass/fail
outcome). The glass/blur visual system is no longer blocked on this gate.

**ADR-0002 spike 2 is now closed** — accessibility validated by the human (VoiceOver on
macOS). See the ADR's "Spike 2 results" section.

**Session 3 (2026-08-14) closed out all three items session 2 had left
open**: OS-secure-storage-backed DB key management (`core/src/keychain.rs`,
via the `keyring` crate — macOS Keychain confirmed working end-to-end, no
prompt/interruption), the Settings pane wired to a real `display_name`
setting round-tripped through the new `settings` table, and `app.slint`'s
hardcoded hex literals swapped for `Theme.*` tokens. All three landed as
separate commits, each verified with full workspace build/test/clippy/fmt
plus a manual run (and, for the theme swap, a screenshot). Pushed to
`origin/main`.

**Session 4 (2026-08-14) tackled the first of session 3's three remaining
steps**: OpenMLS's storage trait is now backed by the SQLCipher DB. Before
starting, the human was asked (a) which of the three remaining steps to do
next and (b) which of two designs for the OpenMLS-storage step — a full
per-field `StorageProvider` reimplementation against SQL rows, or wrapping
`openmls_rust_crypto`'s already-correct `MemoryStorage` and persisting its
whole contents as one blob. Both were genuine forks worth a real decision
(this step is part of the E2EE integration ADR-0004 flags as needing an
independent audit before shipping), not something to guess at silently.
Human picked: OpenMLS storage first, blob-wrap approach. See
`core/src/mls_provider.rs`'s doc comment for the full durability tradeoff —
flagged there as something to revisit before the audit gate if
per-operation durability turns out to matter for real group traffic.
Verified with a real round-trip test (store a `SignatureKeyPair`, flush,
reload into a fresh provider, confirm it reads back), plus full workspace
build/test/clippy/fmt. Pushed to `origin/main`.

**Session 5 (2026-08-14) tackled the P2P scaffolding step**: `core/src/p2p.rs`
now wraps a bound iroh `Endpoint` behind `P2pNode`, using `presets::Minimal`
specifically so this makes no assumption about, and depends on no
availability of, any external discovery/relay service — appropriate given
ADR-0003 itself is still `Proposed` pending its own five spikes (NAT-
traversal measurement, WebRTC/E2EE-through-SFU, group-call topology, an
ANKAI Node reference impl, QUIC-datagram netplay), none of which this
module attempts. Verified with a real round-trip test: two in-process
nodes exchange a message over a direct local QUIC stream, stable across
repeated runs (a first attempt raced a caller's `close()` against the
remote still draining its response stream — fixed by waiting for
`conn.closed()` on the accept side before returning, matching iroh's own
example). Full workspace build/test/clippy/fmt clean. One new fact worth
flagging: adding `iroh` pulls in a few more MPL-2.0/CDLA-Permissive-2.0
transitive deps (`attohttpc`, `webpki-roots`) that `cargo deny check
licenses` rejects, stacking on top of the pre-existing openmls/hpke-rs
MPL-2.0 issue from earlier sessions — not fixed, same ADR-owner licensing
policy call, not an engineering one. Pushed to `origin/main`.

**Session 6 (2026-08-14) tackled the last item from session 5's queue**:
`client/src/main.rs`'s placeholder `AccountId` (which only proved
`ankai-core` was reachable from `client`) is now a real first-run identity-
creation flow. `identity::load_or_create_device` generates a random opaque
account/device id pair plus a real ED25519 `SignatureKeyPair` (via
`openmls_basic_credential`, persisted into `AnkaiMlsProvider`'s storage) the
first time a device's DB is opened, and reloads the same identity — proven
by the *actual private key* still being readable back out of MLS storage,
not just a stored id string — on every run after. `identity::
SignatureKeyPlaceholder` is retired in favor of `DeviceSignatureKey`, which
now wraps a real, storage-backed public key. `AccountId` is still an honest
placeholder in one specific documented way: it's a random string, not
derived from a real account-root identity key, because ADR-0004's multi-
device "Registration" model (an account key that co-signs new devices)
isn't built, and there's no server-side account service yet either — so
"creating an account" can currently only mean "generate a local identity
for this installation." Verified three ways: two new core tests, plus
manually running the real app twice in a row and confirming the Profile
pane shows identical account/device IDs both times (screenshotted both
runs side by side). Full workspace build/test/clippy/fmt clean. Pushed to
`origin/main`.

**Session 7 (2026-08-14) picked candidate 1 above** (asked the human which
of the two to start; they picked KeyPackage generation over UI feature
panes, on the reasoning that it's self-contained and just as verifiable as
the identity work, whereas the UI panes would mostly mean fabricating local
data with nothing real to back it yet). `identity::create_key_package`
builds a real `KeyPackage` via `KeyPackage::builder().build()`, using the
device's already-stored signature key (read back from `AnkaiMlsProvider` as
the `Signer`) and a new explicit `CIPHERSUITE` constant
(`MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`, RFC 9420's mandatory-to-
implement suite, paired with `DEVICE_SIGNATURE_SCHEME`). `build()` itself
persists the key package's private HPKE material into the provider's
storage as a side effect — same flush-after contract as signature key
generation. Deliberately stops at "build and store locally": asked whether
to also wire this into `client` (generate one at startup, show something in
the Profile pane) and the human agreed to keep it core-only for now, since
there's no directory service yet to publish to or anything meaningful to
display. Verified with a real test: build a key package, confirm its
ciphersuite matches, and confirm its leaf-node credential round-trips back
to the exact `BasicCredential` the device was created with. Full workspace
build/test/clippy/fmt clean; no new dependencies. Pushed to `origin/main`.

**Session 8 (2026-08-14) took the "real feature data behind a nav pane"
candidate and picked Communities specifically** (not asked which pane —
Communities was the clear pick since a local-only "create a community you
own" action is genuinely meaningful on its own, unlike Messages/Hangouts
which are inherently about other people and would've meant fabricating
fake conversation/session data with nothing real behind it yet).
`core::communities` adds create/list/get against a new `communities` table
(migration 4); the Communities pane got a real name field + Create button,
backed by main.rs loading the existing list at startup and a
create-community callback that appends to a live Slint list model — same
shape as the Settings pane's display-name field, just list-valued. Along
the way, noticed the "N random bytes → hex string" pattern was about to
appear a third time (already in `keychain` and `identity`), so
consolidated it into a new `core::util` module first rather than
copy-pasting again — a separate, preceding commit, no behavior change.
Verified with a real core test (create two communities, confirm list order
and distinct ids, confirm get() round-trips); the client side compiles
clean and was run/screenshotted, but the create-community *click* path
itself wasn't click-tested on the live desktop, same bar the Settings
Save button was already held to (see this file's synthetic-click note
above). Full workspace build/test/clippy/fmt clean. Also hit and recovered
from an unrelated transient filesystem hiccup mid-session (a `cargo build`
briefly couldn't find the repo's `Cargo.toml`, and one `Write` call for
`communities.rs` didn't persist) — `git status` confirmed nothing was
actually lost once it resolved; the missing file was just rewritten.
Pushed to `origin/main`.

**Session 9 (2026-08-15) ran all three of session 8's candidates in parallel**
(human said "do all of them same time spin up some agents" instead of picking
one). Three isolated git worktrees, one agent per track, each briefed with
explicit scope boundaries so none of them would fake data or silently make an
architectural call that belongs to the human:

- **Messages stub**: `core::messaging` (send/receive plaintext over
  `core::p2p`) + a real Messages nav pane. `p2p.rs` gained a persistent
  `accept_loop` (extending its prior one-shot `accept_and_echo_once`).
  Peer addresses are shared out-of-band as pasted JSON — no discovery yet.
- **ADR-0003 spike 1 tool**: `tools/nat-probe`, a real connect/telemetry
  binary using iroh's `N0` preset (not `core::p2p`'s `Minimal`), verified
  end-to-end with a real localhost smoke test. The agent correctly refused
  to fabricate cohort data — the tool exists, the actual "run it across a
  diverse beta cohort of networks" measurement is still unstarted human
  work, same as it was before this session.
- **Directory-service interface**: `core::directory`'s `DirectoryService`
  trait + in-memory implementation, matching exactly what `identity.rs`
  and `p2p.rs` were already stubbed pending. Deliberately stops short of a
  real server or a new ADR — the agent's own assessment (matching the
  orchestrator's) is that picking real server tech (transport/auth/storage/
  hosting) is genuinely ADR-shaped and shouldn't be defaulted into inside a
  feature branch.

Merged all three onto `main` via three `--no-ff` merges. Only one real
conflict, in `Cargo.toml` (`nat-probe` and `messaging` both added a
`serde_json` doc comment on the same line) — resolved by combining both
comments to describe both consumers accurately. `core/src/lib.rs`'s two new
`mod` lines (`directory`, `messaging`) merged clean, alphabetized.

One process mistake worth flagging for future sessions: the merge commit's
`git add -A` picked up `.claude/worktrees/*` (the framework's own temporary
per-agent worktree dirs) as embedded git repos. Caught immediately by git's
own warning, fixed with a follow-up commit (`git rm --cached`) and a new
`.claude/worktrees/` `.gitignore` entry. **Future sessions: prefer `git add
<specific paths>` over `git add -A` when merging multi-track work in this
repo**, or check `git status` output for unexpected `.claude/` entries
before committing.

Post-merge, full workspace `build`/`test` (28 tests, all pass, including 6
new: 2 messaging/p2p, 5 directory, 4 nat-probe address-parsing minus the 3
that already existed)/`clippy -D warnings`/`fmt --check` all verified clean
on the merged tree — not just per-branch. Ran the real client and
screenshotted it (full-screen capture, no synthetic clicks — see the
standing note below) to confirm no regression and that the Messages nav
entry renders. Pushed to `origin/main`.

**Session 10 (2026-08-15) ran all four of session 9's candidates in
parallel again** (human: "do all of em use agents"). Four isolated
worktrees, one agent per track:

- **MLS-encrypt + persist messaging**: real OpenMLS 2-member-group
  encryption per peer, plus DB persistence (`conversations`/`messages`
  tables). Since there's still no directory service, group setup piggybacks
  a `PeerInvite` (address + a fresh `KeyPackage`) on the same out-of-band
  paste flow the address-only stub already used.
- **Hangouts pane**: local-only hosting metadata (`core::hangouts`),
  mirroring Communities' shape exactly. The agent explicitly declined the
  optional P2P-join stretch goal as scope creep beyond matching Communities'
  precedent — a judgment call, not a shortfall.
- **ADR-0003 spike 2 tool** (WebRTC audio/E2EE prototype): a real two-process
  `webrtc-rs` voice-call prototype, genuinely verified end-to-end (DTLS-SRTP
  active, Opus/RTP round-trip passes). A real connection-timeout bug hunt
  turned out to be a **test-harness artifact**, not a code bug: relaying the
  tool's printed SDP through `zsh`'s `echo` builtin silently rewrites the
  SDP's literal `\r\n` sequences into real newlines mid-string, corrupting
  it in transit (`printf '%s\n'` doesn't have this problem). Worth
  remembering for any future hand-relayed multi-line protocol testing in
  this repo.
- **ADR-0008 + reference backend**: a genuinely researched ADR (axum vs.
  tonic, SQLite vs. Postgres, ED25519-request-signing auth reusing
  `identity.rs`'s existing device key rather than inventing new crypto,
  TOFU pubkey pinning), staying `Proposed` per ADR-0003's own precedent,
  plus a real reference server + a second real `DirectoryService`
  implementation (`HttpDirectoryClient`) proven against each other with
  real signed-request integration tests — including proving rejection
  paths (unsigned, badly-signed, impersonating-a-pinned-device all
  genuinely fail, not just the happy path).

**Process notes for future sessions, some costly this time:**

1. **`isolation: "worktree"` always creates a *new* worktree — it cannot be
   pointed at an existing one.** Passing a specific path in the prompt text
   does nothing if `isolation: "worktree"` is also set; the tool ignores
   the path and the agent lands somewhere else entirely, sometimes
   contradicting its own instructions. To resume a specific existing
   worktree (e.g. after a background agent stalls mid-task with uncommitted
   changes), either (a) launch without `isolation` and pass the exact path
   in the prompt — the agent just `cd`s there via Bash, no pinning involved
   — or (b) if `isolation: "worktree"` is used, treat wherever it lands as
   authoritative and, if prior uncommitted work needs to carry over,
   transplant it with `git diff > patch.diff` / `git apply patch.diff`
   rather than telling the agent to go find the old path — worktree-pinned
   agents' Bash tool can be hard-sandboxed to refuse operations outside
   their assigned directory, including `cd`, so a wrong instruction can
   wedge the agent completely (this happened once this session — recovered
   cleanly by relaunching with the transplant approach instead, no work
   lost, but it cost a full agent turn).
2. **Long-running background builds inside a subagent can cause the agent
   to end its turn prematurely "waiting for a notification" that never
   resumes it.** This happened repeatedly this session (MLS-messaging,
   WebRTC-audio) — an agent ran `cargo build`/`cargo test` in the
   background, printed something like "waiting for the build to finish,"
   and then simply stopped, leaving real uncommitted work behind. Resuming
   via `SendMessage` sometimes worked, sometimes the agent immediately
   re-entered the same stuck pattern. When this happens, it's often faster
   for the orchestrator to just run the verification commands directly
   (`cd` into the worktree, run `cargo build/test/clippy/fmt` in the
   orchestrator's own Bash) rather than keep resuming a wedged agent loop.
3. Mid-session, background agents hit the account's session usage limit
   (all in-flight agents failed simultaneously with the same "session limit
   · resets" error). Nothing was lost — worktrees with uncommitted diffs
   just sat untouched until the limit reset — but it's worth knowing this
   can happen mid-swarm: check `git status` in each worktree before
   assuming an agent's silence means it's still working normally.
4. **Migration-version collisions across parallel tracks are a real,
   expected merge-time cost of this workflow**, not a mistake by either
   agent: Hangouts and MLS-messaging both independently claimed DB
   migration version 5 (each only sees `main` as it existed at launch,
   before the other's migration landed). Resolved at merge time by
   renumbering messaging's migration to 6 and updating the affected test
   assertions — cheap to fix, but always check for this specifically when
   merging multiple tracks that touch `core/src/db.rs`'s `MIGRATIONS` list.

Merged all four onto `main` via four `--no-ff` merges, in the order above.
Real conflicts: `Cargo.lock` (webrtc-audio-probe's merge — resolved by
`git generate-lockfile` after the `Cargo.toml` workspace-members list
merged clean on its own), `core/src/db.rs` (the migration-5 collision above),
`client/src/main.rs` (Hangouts' wiring block vs. messaging's updated module
comment — both real, needed to coexist, not an actual disagreement).

Post-merge, full workspace `build`/`test` (45 tests across `ankai-core`,
`ankai-directory-server` unit + integration, `nat-probe`,
`webrtc-audio-probe`, all pass)/`clippy -D warnings`/`fmt --check` all
verified clean on the fully merged tree. Running the client to screenshot
the final merge hit an unrelated macOS Keychain "dark wake, no UI possible"
/ "unable to obtain authorization" error — a screen-lock/session-state
issue, not a code regression (each track's own agent already
screenshot-verified its pane in isolation before merging, and this exact
Keychain flow has worked in every prior session). Pushed to `origin/main`.

**Session 11 (2026-08-15) ran three more tracks in parallel** (human:
"multiple conversations at once and work on the bigger unstarted parts of
the original idea use agent do em simultaneously"). Also picked up
candidate 4 from session 10's list first, since the human separately asked
for a way to find people by something shorter than a full address blob:

- **Directory server wired into `client` (opt-in)**: publishes this
  device's `KeyPackage`/`EndpointAddr` to a real running directory server
  on startup if `ANKAI_DIRECTORY_URL` is set, and offers "look up by Device
  ID" in Messages as an alternative to pasting a full invite. Off (and
  behaviorally identical to before) if the env var is unset — kept opt-in
  since ADR-0008 is still `Proposed`. Verified for real: a test spawns the
  actual compiled server binary as its own OS process and drives a full
  cross-process lookup + encrypted message exchange through it.
- **Community discussion posts**: the "forum" half of "communities/forums"
  — flat, append-only text posts inside a community.
- **"Top 8 Communities" profile customization**: the classic MySpace
  Top-8 idea, honestly rescoped to communities (not "friends," since ANKAI
  has no real contacts/relationship model yet — explicitly flagged as
  something to revisit once one exists, not something to fake now).

**Multi-conversation messaging UI (session 10's candidate 1) and
usernames-for-the-directory-service (a new human request) were both
deliberately sequenced behind the directory-wiring track** rather than run
in parallel with it — all three would touch the same Messages-pane code in
`client/src/main.rs`/`app.slint`, and conflicting edits there would have
cost more time to untangle than the parallelism saved. Neither has started
yet.

**Process notes:**
- The now-familiar "agent stalls mid-task, ends its turn waiting for its
  own background build instead of actually waiting" failure mode (see
  session 10's notes) recurred for all three tracks this session. Rather
  than keep resuming stuck agents, the orchestrator finished verification
  directly each time (running `cargo build/test/clippy/fmt` itself in the
  agent's worktree) — faster than fighting the loop, and `TaskStop` was
  used once a track's work was already safely committed but its agent kept
  looping and burning tokens regardless.
- One real bug surfaced and was fixed directly by the orchestrator: the
  Top8 track's `client/src/main.rs` had a borrow-then-move conflict
  (`featured_ids` borrowed `&str` out of `featured` right before `featured`
  was consumed) — fixed by collecting owned `String`s instead.
- The human has said explicitly they trust agent-run testing and don't want
  to be a bottleneck for validation — screenshots/manual review still
  happen, but nothing in this session waited on the human clicking through
  anything themselves.

Merged all three onto `main`. Two real conflicts, both in the
already-familiar shape (two branches independently extending the same
Profile-pane wiring in `client/src/main.rs`/`app.slint` with genuinely
independent, non-overlapping features) — resolved by keeping both blocks,
not by picking one over the other. Post-merge, full workspace `build`/
`test` (59 tests across all crates, plus the directory server's
subprocess-based end-to-end test run explicitly)/`clippy -D warnings`/
`fmt --check` all verified clean, plus a real client run/screenshot
confirming no regression. Pushed to `origin/main`.

Remaining steps: no locked queue again. Reasonable next candidates, none
started, none chosen over the others:

1. Multi-conversation messaging UI (queued behind this session's directory
   work, now unblocked) — `core::messaging`'s DB tables already support
   more than one conversation, the UI still only shows one at a time.
2. Usernames for the directory service (human request, also unblocked now)
   — let people claim a short human-readable name instead of sharing a raw
   Device ID. Default scoping call already made: **device-scoped** for now
   (tied to a `DeviceId`, first-come-first-served, ownership enforced via
   the same request-signing the directory server already does), flagged as
   needing revisiting once real multi-device/account support exists.
3. Human-side validation work: ADR-0003 spike 1 (NAT-traversal cohort
   measurement across real diverse networks) and spike 2 (subjective
   AEC/noise-suppression audio quality on real hardware) both have working
   tools now but still need a human to actually run them.
4. The remaining ADR-0003 spikes (3-5: group-call topology, ANKAI Node
   reference impl, QUIC-datagram netplay) — no tooling started on any yet.
5. Other still-unstarted parts of the original product idea: real forum
   threading/replies (posts are flat-only right now), the creator
   marketplace (explicitly held back this session — involves real
   payments/money, a product-and-legal decision the human should weigh in
   on before any scaffolding starts, not something to default into).
6. Frame-level E2E encryption through an SFU (ADR-0003 spike 2's harder
   half) — genuinely unstarted, needs an actual SFU to forward through.

Not a locked decision — say which direction (or several, in parallel again
if that's still the preferred mode), or propose something else.

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
   undone item. If that list presents open candidates rather than an
   ordered queue (true as of session 6 — the original queue emptied out),
   ask the human which to pursue rather than picking unilaterally, the same
   way sessions 4 and 5 did before starting.
5. If the user just says "continue ANKAI" with no other context, that's enough —
   do not re-ask them to re-explain the product.
