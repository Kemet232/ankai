# ANKAI — Progress / Resume Point

> **If you are picking this up cold (new session, ran out of context, etc.):**
> Read this file top to bottom, then skim `docs/adr/*.md` for decisions already locked in.
> That's enough to resume without re-reading the full product spec.

Last updated: 2026-08-16 (session 16)

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
| Phase 1 UI shell rebuild | **done (session 17)** — extracted reusable `AppNavButton` and `TopCommandBar`; slimmer icon-led sidebar; persistent page-aware command bar with global search, notifications, and avatar action; every new action is wired (search routes to Discover and executes, avatar opens My Page, unavailable notifications report an honest transient notice); all screens offset below the command bar; named route indices exposed to Rust instead of duplicated magic numbers | `client/ui/shell-components.slint`, `client/ui/app.slint`, `client/src/main.rs` |
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
| Usernames for the directory service | done — a device can claim a short name (lowercase alphanumeric/underscore, 3-20 chars) via the directory server instead of sharing a raw Device ID; device-scoped (not account-scoped — flagged for revisit once real multi-device/account support exists), first-come-first-served with the same signed-request ownership pinning the server already uses for device keys. Wired end-to-end: claim in Settings, look a peer up by username in Messages. Real full-stack test: device B finds device A purely by username and sends a real MLS-encrypted message through a real running server | `server/directory/src/username.rs`, `client/src/directory.rs`, `client/src/main.rs` |
| Real "Liquid Y2K" visual design applied (shared shell + Profile pane) | done — the validated glass-panel design system (`docs/design/tokens.md`, proven in `client/ui/spike-glass-blur.slint`, gated on two ADR-0002 spikes that closed back in session 3 but were never actually applied to the real app until now) is live on the sidebar/window chrome and the Profile pane: glass-tinted panels, glowing active-nav pill, avatar-initial header card, de-emphasized ID chips, and the Top 8 list rebuilt as real accent-cycled glass module cards with custom icon buttons instead of generic OS buttons. Messages/Communities/Hangouts/Settings pane *content* untouched this pass (shared chrome only) — human asked to see one screen done properly before the rest follow | `client/ui/app.slint` |
| Anime metadata + watchlist (`core::anime`) | done — real AniList GraphQL integration (search/trending/popular, unauthenticated public read, no API key needed — confirmed via live requests, real rate limit is 30/min not the commonly-cited ~90/min) plus a real local watchlist (watching/completed/planned/dropped, episode progress). Real `#[ignore]`d live-network test; caught AniList having a real live outage mid-session and reported that honestly instead of faking a pass | `core/src/anime.rs`, `core/tests/anilist_live.rs` |
| Episode/movie resume persistence | **Phase 5 backend complete; player/UI wiring pending** (session 17) — migration 10 adds an encrypted `playback_progress` table keyed by provider/media/optional episode. Validated finite position/duration upserts, movie and episode keys, newest-first resumable listing, completion, and clearing are covered by five focused tests; the full core suite passes 80 tests with one live test ignored. The player still needs to save periodically/on close and the Home/My Page surfaces still need Continue Watching cards | `core/src/playback_progress.rs`, `core/src/db.rs` |
| Cross-community recent-activity feed | done — `core::forum_posts::list_recent_across_communities`: real posts pulled from every local community into one most-recent-first feed, each correctly tagged with its real community name — the data source for "Hot Discussions" | `core/src/forum_posts.rs` |
| Real friends system + on-demand presence (`core::friends`) | done — real P2P friend-request/accept flow (signed, real crypto, rides the existing P2P transport as a lightweight non-MLS message kind since a request has to work *before* any conversation/group exists), a real `friends`/`friend_requests` DB schema, and genuine presence: `check_presence` makes a real ~3s-timeout connection attempt right now rather than returning a stored flag — proven by a test that closes a node mid-test and watches the same call flip from online to offline. Device-scoped, not account-scoped (same honest limitation as Top 8/usernames) | `core/src/friends.rs` |
| MyAnimeList forum discussions (`core::mal_forums`) | done — real read-only client (boards/topics/topic-posts), Client-ID-only auth (no OAuth login needed — confirmed empirically), reads the real credential from an env var at runtime, never hardcoded. Real live-network test against MAL's actual servers | `core/src/mal_forums.rs` |
| Home dashboard (new screen) | **Phase 2 complete (session 17)** — extracted into a responsive, reusable `HomeDashboard` with wide/compact layouts; neon poster-led hero and horizontal Popular carousel; real AniList images/data, friends/presence, local discussions/Hangouts, and MAL topics; explicit loading/empty/error/retry states; hover/pressed/focus semantics; every visible action routes, retries, changes persisted watch state, or opens the real MAL topic. The older inline layout is retained only as an unreachable one-pass fallback pending mechanical deletion | `client/ui/home-dashboard.slint`, `client/ui/app.slint`, `client/src/main.rs` |
| My Page redesign (tabs) | **precision pass in session 14**: removed a fabricated always-on "Early Adopter" header badge, replaced with a real presence indicator (you're trivially online because this client is the one running — an honest claim, not a network one) plus your real claimed username if you have one; restructured into the reference's side-by-side module rows (Top 8 + Currently Watching, then Guestbook preview + My Charms) instead of a stacked column. Currently Watching now shows real cover art. Real friends list + real pending-request Accept/Decline; Guestbook and My Charms stay honest, clearly-labeled placeholders (no real networked-write path / no real unlock system exist yet); Stats tab shows real counts | `client/ui/app.slint`, `client/src/main.rs` |
| Real in-memory remote image loading (`client::images`) | done — real anime cover art fetched from AniList's own CDN via `reqwest`, decoded via the `image` crate, handed to Slint as a `slint::Image` built from the decoded RGBA buffer. No disk cache, ever (explicit human instruction) — a process-lifetime in-memory `HashMap<url, slint::Image>` only. Wired into Home's Trending Now hero, Popular Right Now, and My Page's Currently Watching. Two real bugs fixed during integration: a Slint `ImageFit` compile error from a `cover` property name colliding with the `ImageFit` enum's own `cover` value, and a `Send`-safety bug (the non-`Send` `slint::Image` can't be built on a background tokio task and handed to `invoke_from_event_loop` — fixed by only sending raw RGBA bytes across the thread boundary and building the `slint::Image` back on the UI thread) | `client/src/images.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| MAL Forum Discussions real post previews | done — each topic card now shows a real excerpt from its actual first post (`core::mal_forums::get_topic_posts`, one extra fetch per topic, loaded progressively), not just title/reply-count. Real MAL post bodies are raw phpBB-style BBCode; a small client-side formatter (not core — matches `core::mal_forums`'s own "rendering post bodies is a UI-layer concern" note) strips tags, drops `[img]...[/img]` content and bare image URLs (both were observed leaking full CDN URLs into the preview text on a real live topic before this was caught), decodes HTML entities, and truncates | `client/src/main.rs` |
| `client/build.rs` rerun-if-changed bug | **fixed** — the build script only ever listed two floating-panel-demo files in `cargo:rerun-if-changed`, and printing any such line replaces cargo's default "rerun on any file change" fallback with "only rerun for what's listed." Edits to `app.slint`/`theme.slint` could silently stop triggering Slint regeneration on an incremental build once a target directory already had cached output — caught for real when a color-palette-only edit compiled clean but rendered the old colors. Fixed by globbing every real `ui/*.slint` file into its own rerun-if-changed line | `client/build.rs` |
| Color palette rework (pink/violet, matching the reference) | done — the reference mockup uses a hot pink/magenta accent plus blue-violet, barely any cyan; ANKAI's palette leaned cyan/violet/gold. Real colors sampled directly from the reference image via pixel-level histogram analysis (not guessed): `#E82888` (a "like" count badge) → `theme.slint`'s new `accent-pink` (`#F0459A`, lightened for AA contrast), and `#6C4CD0` (primary buttons) → deepened `accent-violet` (`#8B6CF0` → `#8363E3`). `accent-cyan` usages replaced app-wide (nav pill, section labels, hero glow, avatar gradients, tab pills, accent-cycle arrays, Now Playing widget) except where cyan was already paired with violet in a gradient | `client/ui/theme.slint`, `client/ui/app.slint` |
| Last.fm "Now Playing" backend (`core::lastfm`) | done — real, read-only client for Last.fm's public `user.getrecenttracks` endpoint (API-key-only, no OAuth needed for this call — confirmed live). Returns `Ok(Some(NowPlaying))` only when Last.fm's own `nowplaying` flag is genuinely set, `Ok(None)` for an honest "nothing playing" (including zero scrobble history), `Err` for a real failure — never fabricated. No elapsed/duration data exists on this endpoint, so the My Page widget's progress bar is an explicitly-decorative indeterminate pulse, not a real scrubber. Wired via a `slint::Timer` periodic refresh (not one-shot); Last.fm username is a local-only Settings field, separate from the API key. Real live test (`cargo test -p ankai-core --test lastfm_live -- --ignored`) confirmed against a real public account and a real nonexistent-username 404 | `core/src/lastfm.rs`, `core/tests/lastfm_live.rs`, `client/ui/app.slint`, `client/src/main.rs` |
| Floating draggable-window component (standalone demo only) | done as a real, working component — a reusable `FloatingPanel` (glass-styled, real drag-to-move via the established TouchArea-origin-capture technique, real close callback, "bring to front" via a z-order-hint pattern) with its own demo entry point (`cargo run -p client -- --floating-demo`), verified with a real synthesized OS-level drag that moved a panel on screen. Not wired into the real app directly (see next row — a non-draggable sibling was wired in instead) | `client/ui/floating-panel.slint`, `client/ui/floating-panel-demo.slint` |
| Fixed-position glass panels wired onto Home/Messages/Hangouts | done (session 15) — new `FixedPanel` component (non-draggable sibling of `FloatingPanel`, same glass tint/border/glow/title-bar styling, laid out via normal Slint stretch/fill instead of free-floating coordinates). Wraps Home's Friend Activity/Popular Right Now/Hot Discussions/Watch Parties/MAL Forum Discussions sections; Messages split into "Connect a Peer" + "Conversation" panels (pane now scrollable); Hangouts split into "Create a Hangout" + "Your Hangouts" panels (also now scrollable, with an honest empty state). Trending Now hero left as-is (full-bleed cover art, already has equivalent glass-card styling). No drag mechanism wired onto any real pane, per standing instruction | `client/ui/floating-panel.slint`, `client/ui/app.slint` |
| Stremio addon discovery | done (session 16) — typed live manifest/catalog/meta/stream client; configured manifest paths preserved; multiple catalog and stream-only addons can be loaded together; searches merge catalog providers and stream selection queries every compatible stream provider; WebP poster art loads into Slint in memory | `core/src/stremio.rs`, `client/src/main.rs`, `client/ui/app.slint` |
| Stremio addon management — Phase 3 backend checkpoint | **backend done; UI wiring still pending** — `core::addons` persists a versioned ordered registry in the existing SQLCipher-backed `settings` table (no new migration): canonical configured manifest URL, identity/display metadata and logo URL, content types, catalog/meta/stream/subtitles roles, enable/disable state, normalized priority, and last-known health/error. Add/remove/reorder, manifest refresh and failure-state operations have focused unit coverage. Configured path variants remain distinct even when they declare the same addon id; equivalent base/manifest URLs are deduplicated | `core/src/addons.rs`, `core/src/stremio.rs` |
| libmpv playback | **Phase 4 backend complete; full control overlay wiring in progress** (session 17) — dynamically loads bundled libmpv and renders through its OpenGL API; now owns a typed playback state machine, safely drains copied events without blocking the UI, observes duration/position/buffering/pause/mute/volume/speed/seeking/EOF/errors, discovers audio/subtitle/video tracks, exposes validated seek/volume/speed/track controls, and includes a `render_to_fbo` seam for a bounded video surface. Seven focused playback tests pass. The Slint transport overlay and final bounded-FBO plumbing are the remaining Phase 4 work; Windows/Linux packaging and LGPL artifact QA remain unverified | `client/src/playback.rs`, `client/src/main.rs`, `scripts/bundle-libmpv.sh` |
| Animated loading experience | done (session 16) — original full-proportion violet/pink virtual-idol mascot generated for ANKAI and animated in Slint with a looping bounce/sway/glow while addon manifests, searches, and stream lists load | `client/ui/assets/ankai-loading-idol.png`, `client/ui/app.slint` |

The old glass/blur + drag-reorder spike still exists (now themed via
`Theme.*`) at `client/ui/spike-glass-blur.slint`, reachable only via
`cargo run -p client -- --spike` (or `ANKAI_SPIKE_DEBUG=1`) — it is not
the default UI anymore.

## Critical product audit (session 16)

ANKAI now demonstrates many real subsystems, but it is not yet a coherent
shippable social product. Its strongest differentiators are the native visual
identity, local-first encrypted architecture, combined anime discovery/social
surface, and now real in-app media playback. Its main weakness is breadth:
many features exist as technically honest vertical slices, while the everyday
loops that make users return are still discontinuous.

### Highest-impact gaps

1. **The core user loop is fragmented.** Discovering a title, adding it to a
   watchlist, starting playback, inviting friends, entering a Hangout, and
   discussing the episode are separate surfaces with little shared context.
   The next product milestone should be one end-to-end title detail screen
   that connects all of them.
2. **Playback is an engine integration, not yet a player.** Add elapsed time,
   duration/seek, volume/mute, fullscreen, buffering/error state, subtitle and
   audio-track selection, episode navigation, resume position, keyboard
   shortcuts, and mpv property/event observation. Torrent streams still need
   the explicit resolver boundary; do not present them as playable before one
   exists.
3. **Addon management is session-only and under-governed.** Persist installed
   addon URLs, provide enable/disable/remove/reorder, show declared resources
   and content types, distinguish catalog/metadata/stream/subtitle roles, add
   per-addon health/error state, and introduce an explicit network-permission
   prompt. User-supplied addon URLs currently create a broad outbound-network
   capability; localhost/private-network access needs a deliberate policy so
   self-hosted addons remain possible without silently enabling SSRF-style
   behavior.
4. **Identity is not an account system.** Device-scoped IDs/usernames/friends
   cannot deliver multi-device continuity, recovery, account portability, or
   reliable discovery. This is the largest architectural product dependency
   and must be designed together with the E2EE recovery model.
5. **Messaging/community depth is below the visual promise.** There is no
   conversation list, unread state, delivery/read state, attachments, replies,
   editing, moderation workflow, blocks, reports, or real community membership.
   The polished shell currently makes these omissions more noticeable.
6. **Hangouts are still labels, not watch parties.** They need membership,
   invites, host authority, synchronized playback state, reconnect behavior,
   voice/chat integration, and clear behavior when participants use different
   stream sources.
7. **Data resilience is thin.** Third-party reads are mostly live-only with no
   pagination, rate-limit coordination, stale-while-revalidate cache, retry
   policy, cancellation, or offline rendering. Images have a process cache but
   no bounded eviction. Slow calls can outlive a changed screen/query.
8. **Release engineering is unfinished.** The project can load a development
   Homebrew libmpv, and has a bundle-copy hook, but CI does not yet build and
   attest LGPL-only libmpv/FFmpeg artifacts for macOS, Windows, and Linux.
   Code signing, notarization, updater behavior, crash reporting, migrations,
   and reproducible release manifests are not established.
9. **Trust, safety, and legal controls lag extension capability.** Before a
   public addon/provider directory, ANKAI needs source attribution, permission
   review, block/report paths, content-policy enforcement, malicious-manifest
   handling, URL/redirect limits, and a clear distinction between public
   protocol compatibility and endorsement of particular content sources.
10. **Accessibility and performance need re-validation after the redesign.**
    Earlier Slint spikes passed, but the new background, large remote-image
    grids, animated loader, 60fps video redraw, and overlay controls materially
    changed the workload and focus structure. Re-run keyboard-only, VoiceOver,
    reduced-motion, contrast, low-end GPU, memory, and long-session tests.

### Recommended execution order

1. Finish the player UX and title-detail/episode flow.
2. Persist and govern addons; add cancellation, caching, pagination, and errors.
3. Turn Hangouts into a real synchronized playback room using the finished
   player boundary.
4. Complete conversation navigation and community membership/moderation.
5. Decide and ADR the real account/multi-device/recovery system.
6. Build reproducible signed cross-platform releases, then repeat performance,
   accessibility, security, and licensing validation before any public beta.

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

**Session 12 (2026-08-15) started with two more parallel tracks (usernames,
then a UI redesign), then paused mid-flow at the human's request.**

- **Usernames for the directory service**: built as scoped in session 11's
  notes (device-scoped, first-come-first-served, same signed-request
  ownership pattern as device-key TOFU pinning). Verified with a real
  full-stack test (device B finds device A purely by username, sends a
  real MLS-encrypted message through a real running server) — the
  orchestrator wrote an equivalent test concurrently with the agent by
  accident (both working in the same worktree at once) and kept the
  agent's version, which used a cleaner dedicated helper.
- **UI redesign**: the human pushed back hard mid-session — "i really
  dont like how basic the ui is... its not what i had in mind at all and
  is not intuitive." Investigation found the real cause: ANKAI has a
  detailed intended visual design (`docs/design/tokens.md`, "Liquid Y2K" —
  dark glass panels, cyan/violet/gold accents) that was validated for real
  back in session 3 (ADR-0002's two spikes both closed), but nobody ever
  applied it to the actual app — every feature session since then
  (including all of this one) just built on the plain flat nav skeleton
  that was only ever meant to be a temporary placeholder. Human wanted one
  screen redone properly before committing to the rest; the shared window
  chrome (sidebar/background) and the Profile pane got the real glass
  treatment (see phase table above). Messages/Communities/Hangouts/Settings
  pane content is still the old plain style, waiting on the human's
  reaction to Profile before continuing.

**Process notes:**
- The stuck-agent-ends-turn-mid-background-build pattern (see sessions 10
  and 11) recurred for both tracks again; handled the same way — the
  orchestrator finished verification directly rather than fighting the
  loop, and used `TaskStop` once a track's work was safely committed but
  its agent kept looping and burning tokens regardless.
- **Twice this session, a screenshot meant to capture the running ANKAI
  app instead captured an unrelated browser window** (the human's own
  browsing) even though `osascript` reported the correct window bounds
  immediately beforehand. Both screenshots were deleted immediately and
  not examined/kept/referenced further; no further live-app screenshots
  were attempted after the second occurrence — verification fell back to
  source review plus one earlier screenshot that *did* legitimately
  capture the app (taken before this failure mode started occurring).
  **Future sessions: treat window-bounds-then-screencapture as
  unreliable in this environment specifically** — a successful
  `osascript` bounds query does not guarantee the app window is actually
  the topmost thing on screen at capture time. If a screenshot looks
  wrong (unrelated content, wrong app), delete it immediately and stop —
  do not retry blindly, and do not describe or act on what was captured.
- A live UI click (attempting to open the Communities pane to see a
  populated Top 8 card) also missed the app window and landed on an
  unrelated browser tab, for the same underlying reason. Stopped
  attempting further live interaction immediately, per this repo's
  standing caution (see the Hangouts-era incident note elsewhere in this
  file) — verified the populated-card styling via source review instead.

Merged both tracks onto `main` — no conflicts on either merge. Full
workspace `build`/`test` (77 tests)/`clippy -D warnings`/`fmt --check` all
verified clean on the merged tree. Pushed to `origin/main`.

**Session 13 (2026-08-16) picked up right where session 12 paused** — the
human came back with a generated reference image showing a much fuller
"anime social network" vision (a Home dashboard, a richer My Page, an
MSN-Messenger-style floating-window aesthetic) and said to replicate it and
build whatever backend it needs. Agreed approach: **layout first, real
systems after**, with honest placeholders only where a real system
genuinely doesn't exist yet (never fake data).

**Backend systems built for real this session** (see phase table above for
each): `core::anime` (AniList + local watchlist), cross-community recent
posts, `core::friends` (real P2P friend requests + genuine on-demand
presence — not a placeholder), `core::mal_forums` (MyAnimeList forum
discussions). Then two real UI screens on top: **Home dashboard** (new) and
a tabbed **My Page** redesign. Plus a real, working floating-draggable-panel
component (`FloatingPanel`) — built as a standalone demo since the human
changed their mind mid-session about needing actual drag interaction (see
below), so it's proven-real but not yet wired into the real app.

**Reddit "Around the Web" stayed blocked all session, worth remembering.**
The human tried registering a real Reddit developer app live, in the
browser, with the orchestrator's help — hit a genuinely broken/gatekept
registration flow (CAPTCHA loops, a "Responsible Builder Policy" wall, an
automated-account username/password step the orchestrator correctly refused
to fill in). A Medium post pushing a third-party Reddit-scraping service
("FetchLayer") was evaluated and explicitly rejected — it works by ignoring
Reddit's own `robots.txt`/access policy, which is exactly the kind of
workaround this project won't quietly build around. Pushshift was checked
too — dead for general use since 2023, mod-only now. **MyAnimeList was
chosen instead** (real forums, real free Client-ID-only API, an actually
working registration flow) — see `core::mal_forums` above. **The human has
since dropped Reddit entirely** — not a future candidate, not queued, done.
Don't propose it again without the human raising it fresh.

**The "floating window" scope changed twice, worth remembering exactly
what was decided:** the human first asked for a full MSN-Messenger-style
draggable floating-window paradigm ("idc how you do it"); a real, working
`FloatingPanel` component was built and verified with a genuine synthesized
OS-level drag (the panel visibly moved on screen). Then the human changed
their mind: **no drag needed, just a fixed layout matching the reference
image.** So the built `FloatingPanel` component (with real drag logic) is
more capability than currently wanted — wiring the *fixed-position* version
onto the real Home/Messages/Hangouts panes (reusing the same glass-panel
visual work, dropping the drag mechanism) is now a smaller task than
originally scoped. Don't rebuild drag support unless asked again.

**Process notes:**
- Hit the account session-usage limit again mid-session (see sessions 10-12
  for the same recurring pattern) — three UI agents were cut off mid-task.
  Recovered the same way as before: verified/fixed each directly rather
  than re-running agents from scratch.
- **A new failure mode this session, worth flagging clearly for future
  sessions:** two agents launched via `isolation: "worktree"` back-to-back
  ended up branching from a **stale** snapshot of `main` — missing three
  backend merges (`core::anime`/friends/cross-community-posts) that had
  just landed moments earlier. Both agents independently noticed the
  missing modules and recreated near-duplicate copies of them to unblock
  themselves, rather than failing outright — which meant the orchestrator
  had to `git merge main` into each agent's branch by hand afterward
  (checkpointing the agent's own uncommitted work first) and resolve real
  add/add conflicts on the duplicated files, keeping the already-merged
  `main` copies as authoritative. **If a freshly-launched worktree's
  `git log`/`git merge-base` shows it's missing recent `main` commits you
  know just landed, don't assume it's current — check explicitly before
  trusting an agent's report that a dependency "already exists."**
- **Component/function name collisions across independently-built UI
  tracks are a real, recurring merge cost** (third time this pattern's hit
  this project — see sessions 9-10's notes for the DB-migration-version
  version of the same problem). This session: two agents both independently
  named a Slint component `FriendCardItem` (different shapes) and a Rust
  function `refresh_friends` (different signatures/purposes) for genuinely
  different reasons. Resolved by renaming one side's to a track-specific
  name (`HomeFriendCardItem`, `refresh_home_friends`) rather than trying to
  unify them — they really were different things. When a merge conflict
  looks unusually tangled (interleaved, not simple add/add), it's often two
  independently-added components/functions with the *same name* confusing
  the diff algorithm, not real disagreement about the same code — check for
  that specifically before trying to hand-merge line by line.
- One real compile error (`E0716`, a temporary-value-dropped-while-borrowed
  in a friends-presence refresh closure) and one real `cargo fmt` violation
  were both caught and fixed directly during merge reconciliation — both
  were things the affected agents' own verification never reached because
  they were cut off by the session limit first.
- Live drag-testing the `FloatingPanel` demo required synthesizing a real
  OS-level mouse drag — no `cliclick`/`pyautogui`/pyobjc-`Quartz` were
  available on this machine, so this session wrote a small ad hoc
  `ctypes`-based Python script calling macOS's `ApplicationServices`
  `CGEventCreateMouseEvent`/`CGEventPost` directly. That script (not saved
  anywhere permanent — scratchpad only) is a real, reusable technique if a
  future session needs to test actual drag/multi-step-gesture UI
  interactions on macOS again.
- The window-capture-grabs-the-wrong-content failure mode (documented in
  session 12's notes and in memory) recurred once more this session during
  the drag test — deleted immediately, not described, consistent with the
  established response.

Full workspace `build`/`test` (102 tests)/`clippy -D warnings`/`fmt --check`
all verified clean on the fully merged tree, and the real app was compiled
and launched for the human at the end of the session, per their request to
"finish for today."

**Session 14 (2026-08-16) ran three parallel agents** to close the human's
standing complaint that "the UI looks nothing like the reference" and that
real images/MAL forum posts weren't visible yet: real in-memory remote image
loading, the Home dashboard grid rebuild (+ wiring in MAL forums, missing
since session 13), and a My Page precision pass (removing a fabricated
badge, matching the reference's two-column module rows). All three were cut
off mid-task by the recurring session-limit/stuck-agent pattern (see
sessions 10-13) and finished by the orchestrator directly: diagnosed and
fixed two real bugs the image-loading agent left unresolved (a Slint
`ImageFit` compile error, a `slint::Image`-isn't-`Send` bug — see phase
table above for both), then ran full `build`/`test`/`clippy -D warnings`/
`fmt --check` on each worktree individually before merging.

**Merging three branches that all touched `client/ui/app.slint` and
`client/src/main.rs` produced real conflicts**, resolved using the same
"reconstruct from clean per-branch `git show` extracts" technique
documented in session 13's notes (the interleaved-diff problem recurs
whenever two branches restructure the *same* region of the file rather than
adding non-overlapping content — not a sign of real disagreement, just the
diff algorithm getting confused). One real signature-mismatch bug surfaced
by the merge itself (not caught by either agent, since neither had the
other's changes): `on_watch_now`'s callback still called `refresh_watchlist`
with the old two-argument signature after the image-loading branch added a
required `tokio::runtime::Handle` parameter for kicking off cover fetches on
watchlist refresh — fixed by capturing a cloned handle into the closure the
same way `db`/`app_weak` already were.

**Verified visually end-to-end after merging**: launched the real app,
confirmed frontmost via `osascript` before every capture (hit the
documented wrong-window failure mode once more — `osascript` reported
"Terminal" as frontmost right after a click un-focused the app window;
deleted that capture immediately without viewing it, re-activated, and
re-verified frontmost immediately before *and* after the retry). Home shows
real AniList cover art loading into the Trending hero and the Popular Right
Now scroll row, and real MAL Forum Discussions with genuine topics/reply
counts. Clicking "Watch Now" on the Trending hero card genuinely adds the
anime to the real watchlist and it shows up with real cover art on My
Page's Currently Watching module — the full pipeline works, not just each
piece in isolation.

**Also registered a real, free Last.fm API account** (human's own account,
human completed the CAPTCHA/submit themselves per the standing
never-touch-bot-checks-or-credentials rule) as the "now playing" data source
after Spotify was confirmed blocked (Premium-account requirement, verified
live in session 13) and YouTube was confirmed to have no official API. The
resulting API key + shared secret were saved directly to
`~/.ankai_lastfm_credentials.env` (chmod 600, outside the repo, values never
echoed into chat/any repo file — same handling as the existing MAL
credentials file) and are ready for a `core::lastfm` backend track
whenever that's picked up; nothing built against it yet this session.

**Session 14 continued** after the human reviewed the above and asked for
three more things directly: real cover art on the Trending Now hero card
(it was text-only), real excerpts on MAL Forum Discussions cards ("not just
the title... like a lil preview"), and to run more agents in parallel. Two
were fixed directly (hero cover art, MAL preview text — see phase table),
surfacing two real bugs along the way: `[img]...[/img]`/bare image URLs
leaking raw CDN links into the MAL preview text (fixed with a small
BBCode-to-plain-text formatter in the client), and — much bigger — a real
`client/build.rs` bug where `cargo:rerun-if-changed` only ever listed two
floating-panel-demo files, so **incremental builds could silently render
stale UI code after a `.slint`-only edit with no compile error to catch
it**. Found while verifying a color-palette agent's work looked unchanged
in a real screenshot despite correct source; fixed by globbing every real
`ui/*.slint` file into its own rerun-if-changed line (see phase table). Any
future session doing `.slint`-only edits should know this was a real,
previously-live footgun, not assume every visual check tonight before the
fix was trustworthy for pure-Slint changes (Rust-visible API changes, e.g.
new struct fields, were self-checking — a stale build.rs would have thrown
a real compile error for those, and none did).

Three more parallel agents were launched (color palette rework, a real
`core::lastfm` backend, wiring the fixed-position floating panels) — two of
the three hit the account session limit mid-task (a continuation of the
recurring pattern from sessions 10-13) and were finished/verified directly
by the orchestrator, same recovery pattern as every other time. **Color
palette** and **Last.fm backend** are both done, verified (build/test/
clippy/fmt clean, plus a real live-API test for Last.fm), and merged into
`main` — see phase table above for both. **Floating panels did not land**:
that worktree branched from a stale `main` snapshot from *before* this
entire session-14 round (missing 9+ commits), and its real progress when
cut off was ~40 lines of setup (an import, duplicate cover-art properties
already superseded by the real image-loading work) with no actual panel UI
built yet — not worth reconciling against that much drift for that little
real progress. Abandoned rather than merged; still genuinely queued for a
future session to build fresh against current `main` (which already has
real cover art, so it won't need to duplicate that part).

**Also hit a real, isolated macOS permission failure** late in this round:
`osascript -e 'tell application "System Events" to click at {x,y}'`
started failing with "osascript is not allowed assistive access" for the
click-at-coordinates form specifically — `keystroke` and frontmost-name
queries via the same `tell application "System Events"` block kept working
fine, so this wasn't a full permission revocation, just that one command
form. Not resolved this session; the Now Playing widget's *rendering* was
therefore verified via code review + the real live API test + unit tests
rather than a live screenshot of My Page with a configured username — worth
a real click-through screenshot next session once this is sorted out (try
re-granting Terminal/whatever host process Accessibility access in System
Settings, or fall back to the Claude-in-Chrome-style `computer` tool's
click primitive if working in a context that has it, rather than raw
`osascript click at`).

**Session 15 (2026-08-16) ran the floating-panels track** (candidate 1 from
session 14's list) via a single parallel agent, isolated worktree, branched
correctly off current `main` this time (no stale-base repeat of session 14's
failure). Built `FixedPanel` + wired it onto Home/Messages/Hangouts — see
phase table above. Merged clean (`--no-ff`, no conflicts), full workspace
`build`/`test`/`clippy -D warnings`/`fmt --check` re-verified on the merged
tree by the orchestrator (not just trusted from the agent's own report).
Also found and flagged for cleanup a genuine leftover from session 14: an
abandoned worktree (`.claude/worktrees/agent-a4107902cb0e9fb31`, ~12 lines
of uncommitted diff) matching PROGRESS.md's own account of that session's
stale-`main` floating-panel attempt — now fully superseded by this session's
real implementation.

**Stremio addon support was proposed as a new track and scoped down after a
real disagreement, worth recording exactly:** the human initially asked for
full Stremio-addon-store parity including in-app magnet/torrent playback.
Declined building that specific piece — Stremio's addon ecosystem is
dominated by torrent-scraper addons for copyrighted content, and bundling a
download-and-stream torrent engine into a public, real-identity-attached
repo is materially different from a generic dual-use torrent client; this
is a hold, not a preference, and doesn't move with re-asking. Landed on a
scope consistent with ADR-0005's own precedent (never bundle the
risky/legally-loaded component — shell out to something external instead):

- Real Stremio addon protocol client (manifest/catalog/meta/streams) —
  in scope, same shape as the existing MAL/AniList clients.
- In-app playback via libmpv (already ADR-0006's chosen engine) for any
  stream result that's a direct HTTP/HLS/DASH URL — full in-app experience,
  no compromise, including for legitimate addons and self-hosted
  (Jellyfin/Plex-style) addons serving content the human actually owns.
- A `StreamResolver` trait as a genuine extension point for anything libmpv
  can't play directly (magnet URIs etc.) — default implementation is a
  no-op/OS-handoff fallback; the human plans to implement their own resolver
  and register it in `client/src/main.rs` themselves. Not yet started as of
  this update — was about to be launched as a parallel track when the human
  pivoted to asking about iOS + accounts instead; still queued.

**iOS port and a real account system were raised as new asks this session,
both flagged as needing a human decision before agents start, not yet
scoped or started:**
- **Accounts**: `identity.rs`'s `AccountId` is still an honest local-only
  placeholder (see ADR-0004 notes above) — there's no server-side account
  service, no multi-device "Registration" model, no signup/login flow.
  Building real accounts means picking an auth mechanism (password+email?
  magic link? OAuth?) and account-server hosting/tech (likely reusing
  ADR-0008's axum+SQLite reference-backend shape), and it touches identity
  issuance directly enough to matter for ADR-0004's E2EE audit gate — this
  is genuinely ADR-shaped, same bar as directory-server tech and the
  marketplace, not something to default into silently.
- **iOS port**: Slint has an experimental iOS backend, so technically
  plausible, but real device testing/distribution needs Xcode + an Apple
  Developer Program enrollment (paid, $99/yr) for anything beyond
  local-simulator use — what "alpha" means for iOS (own-device testing only
  vs. TestFlight-distributable) changes the scope materially and is the
  human's call.

Remaining steps, none locked:

1. Launch the Stremio addon-protocol-client track (scoped above) — ready to
   go, was paused mid-launch by the human's own request to pivot.
2. Scope and launch the accounts system, once the human answers the
   auth-mechanism/hosting questions above.
3. Scope and launch the iOS port, once the human answers the
   alpha-target-vs-real-distribution question above.
4. Communities/Settings panes still have the old plain content styling
   inside their (now glass-wrapped, as of this session) chrome.
5. Multi-conversation messaging UI — still hasn't started (queued since
   session 11).
6. Human-side validation work: ADR-0003 spike 1 (NAT-traversal cohort
   measurement) and spike 2 (subjective audio quality) both have working
   tools now but still need a human to actually run them.
7. The remaining ADR-0003 spikes (3-5), the creator marketplace (explicitly
   held back — real payments/money, needs a human product/legal decision
   first), frame-level E2E encryption through an SFU — all still
   unstarted, same as before.

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
