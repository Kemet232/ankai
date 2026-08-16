# ANKAI release-readiness audit

Audit snapshot: 2026-08-16. Scope: reachable Slint UI, Rust callback wiring,
remote image/addon boundaries, embedded libmpv behavior, startup degradation,
and the no-install libmpv packaging path.

Run the repeatable checks from the repository root:

```sh
scripts/check-release-readiness.sh
scripts/check-release-readiness.sh --full
scripts/check-release-readiness.sh --full --strict
```

`--strict` is the release gate: it promotes every automated readiness warning
to a failing exit status. The libmpv artifact itself has a separate closure and
loader gate; run `scripts/verify-libmpv-bundle.sh BUNDLE_ROOT EXECUTABLE` against
the actual distributable, not a development binary.

## Severity

- **P0**: blocks a distributable build because it enables native-code/data
  compromise, data loss, or makes the primary browse/play path unusable.
- **P1**: must be fixed before public beta; material accessibility, security,
  reliability, product-scope, or playback-correctness defect.
- **P2**: polish or efficiency work that can follow a guarded beta.

## Current result

No P0 was confirmed by this static audit. This is not yet release-ready: the
open P1 items below remain gates, and an actual bundled artifact has not been
verified by this audit. The latest `--full` run completed formatting, client
compile, strict clippy, and 103 core tests (102 passed, one live-network test
ignored) with zero failures and zero automated warnings. The packaged-artifact
proof below is the remaining manual P1 gate.

### Confirmed/fixed

- **Callback contract — fixed.** Every callback exported by `AppWindow` has a
  corresponding `app.on_*` handler in `client/src/main.rs`. The checker verifies
  this mechanically. This proves that visible controls do not disappear into an
  unregistered Rust boundary; it does not claim each behavior is product-complete.
- **MAL forums — fixed.** The retired `mal_forums` module is deleted and no
  longer registered or referenced by the reachable client/core surface.
- **Jikan scope — fixed.** The reachable Jikan model now contains only an
  internal lookup id, title (needed to associate a score and hand the selection
  to the addon search), and formatted rating. The UI has no Jikan/MAL poster,
  synopsis, type, year, episode, or status fields and explicitly says that
  artwork/details come from addons (`client/ui/anime-discovery.slint:1`,
  `client/src/main.rs:1125`). Complete Jikan responses remain transiently in
  memory only to map a selected rating result back to its title; none of their
  other fields is displayed or persisted.
- **Truthful placeholders — fixed.** Local-only Hangouts and unavailable chat
  explicitly say they are local-only (`client/ui/app.slint:2321`), while the
  Nyaa surface explicitly describes per-upload comment links instead of claiming
  to fetch comment bodies.
- **Player input semantics — substantially fixed.** The full player supplies
  focusable play/seek/volume/track controls, Enter/Space activation, arrow-key
  seeking/volume, Escape close, labels, slider values, and accessible actions
  (`client/ui/player-overlay.slint:26`, `:170`, `:302`, `:586`).
- **Player lifetime — fixed.** `Player::drop` frees the mpv render context before
  terminating the mpv handle (`client/src/playback.rs:1163`), and Slint rendering
  teardown drops the player (`client/src/main.rs:1740`).
- **Missing libmpv does not crash the UI — fixed.** Renderer/player construction
  errors leave the app open and render failures now reach `player-error`
  (`client/src/main.rs:1661`).
- **Player state presentation — fixed.** The secondary buffered-duration line is
  hidden instead of copying the playhead, bounded rendering supplies a real
  frame-ready signal, render failures reach the overlay, and closing playback
  exits fullscreen and clears the video surface (`client/src/main.rs:527`,
  `:1674`, `:3202`).
- **Direct stream protocol boundary — fixed.** `Stream::source` now accepts only
  credential-free public HTTPS URLs, rejects local/native mpv protocols, and has
  focused regression tests (`core/src/stremio.rs:590`, `:983`).
- **Stremio response budgets — fixed.** Addon calls now have connect/request
  timeouts, same-origin bounded redirects, per-resource byte limits, bounded
  error excerpts, and post-deserialization collection/string/extra-tree limits
  (`core/src/stremio.rs:92`, `:252`, `:651`). Literal private IPs, credentials,
  HTTP, and obvious local hostnames are rejected. Hostnames resolve inside the
  reqwest connector through a resolver that returns only public addresses, and
  environment proxies are disabled (`core/src/stremio.rs:45`).
- **Image resource budgets — fixed.** Remote artwork now uses public HTTPS and
  same-origin bounded redirects, connect/request/body/dimension/pixel/allocation
  limits, worker-thread decode, 1600 px downsampling, in-flight URL coalescing,
  and a decoded-byte/item-bounded LRU (`client/src/images.rs:24`, `:150`,
  `:287`, `:373`). The image client uses the same public-address DNS resolver
  and disables environment proxies. Tests cover URL rejection, streamed-body
  limits, pre-decode dimensions, downsampling, and LRU eviction.
- **Reachable keyboard navigation — fixed.** The nav/avatar, Home, title detail,
  Profile, community, Hangout and Stremio catalog controls now have button
  roles, labels, accessible actions, keyboard activation, visible focus, and a
  compact 72 px rail under 900 px (`client/ui/shell-components.slint:4`,
  `client/ui/app.slint:855`).
- **Reduced motion — fixed.** The persisted preference now feeds the shared
  theme, and every Slint file with an `animate` declaration either disables its
  transition or stops its choreography timer when reduced motion is enabled
  (`client/ui/theme.slint:73`, `client/ui/app.slint:856`,
  `client/ui/player-overlay.slint:648`). The checker enforces coverage across
  all animated Slint files.
- **Hollow product affordances — fixed.** The unimplemented notification button
  and Guestbook tab, plus the unearned always-on Early Adopter charm, were
  removed from the reachable UI rather than presented as active features.
- **Legacy pane responsiveness — fixed.** Profile, Messages, Communities, and
  Settings are scrollable, Settings uses compact padding, and the known 340/500
  px controls now clamp to their parent width. Player and Hangout surfaces can
  shrink inside the 560 px compact-shell case; their secondary controls already
  collapse behind width breakpoints.
- **Stale async UI results — fixed.** Independently replaceable Stremio search,
  detail, media art, addon state, Home, watchlist, Jikan, Nyaa, Letterboxd, and
  resume surfaces now issue generation tokens and discard completions that no
  longer own the displayed model (`client/src/main.rs:107`, `:132`). Image
  callbacks also verify the generation plus stable item id/URL before updating
  a row. Obsolete network work is not yet aborted; that efficiency item is P2.
- **Nyaa feed bounds — fixed.** Nyaa has a 15-second timeout, 2 MiB streamed-body
  ceiling, result/title/query bounds, and exact HTTPS host/path validation
  (`core/src/nyaa.rs`).
- **Startup degradation — fixed.** The client has no fatal startup `expect`
  paths. DB/keychain/runtime failures return actionable platform errors, while
  MLS/P2P/invite failures disable only peer controls with an in-app explanation;
  catalogs, images, addons, ratings, resume and playback remain available.

## Open P1 findings

### P1 — the no-install player promise is not proven by an artifact yet

The runtime loader correctly prefers libraries beside the executable but still
falls back to Homebrew/system paths for development
(`client/src/playback.rs:1748`). The bundler/verifier now checks LGPL
attestation, dependency closure, hashes, rpaths/install names, and a real loader
probe, but this audit was not given a final `.app`/Linux/Windows bundle to test.

**Required fix:** produce each distributable in a clean packaging job, run
`verify-libmpv-bundle.sh`, launch with package-manager libmpv paths unavailable,
play a known HTTPS fixture, and archive the receipt/attestation/license inventory
with the release. Failure of that job is a release blocker.

## Open P2 findings

- Generation checks prevent stale async results from reaching Slint, but
  superseded HTTP requests and image decodes still run until completion or their
  timeout. Add cancellation to reduce bandwidth/CPU and cap concurrent distinct
  image/search requests under rapid input.
- The 16 ms redraw timer correctly stops after a player error, but still runs for
  the entire time a healthy player is active, including pause/static frames
  (`client/ui/app.slint:823`). Drive redraw from mpv's render update callback or
  throttle paused/background playback.
- Resume poster matching uses title text rather than the playback key; duplicate
  titles can receive the wrong image. Match provider/media/episode identity.
- Home discussion/friend clicks navigate only to a broad destination and show a
  toast rather than selecting the exact row/community. Preserve the requested
  destination and focus it after navigation.
- Long backend error strings are shown directly. Keep full detail behind a
  disclosure/copy action and show a short, actionable message by default; redact
  configured addon URL path/query secrets.
- Nyaa and Letterboxd open links through a generic platform helper. Keep their
  already validated exact URLs, and add defense-in-depth host checks immediately
  before spawning the browser; avoid Windows shell parsing for URLs.

## Manual release matrix

The following checks are still required even when the automated gate passes:

1. **Clean first launch:** empty app-data directory; keychain allowed, denied,
   locked, and unavailable; offline; P2P port unavailable; libmpv absent.
2. **Navigation:** mouse, keyboard only, VoiceOver, NVDA; all destinations;
   focus retained after async updates; 200% text; reduced motion on.
3. **Layout:** 560×360, 800×600, 960×640, 1440×900; resize during loading and
   playback; long display names/addon titles/errors; empty/one/maximum lists.
4. **Networking:** slow, offline, timeout, 429, 404, invalid JSON/XML, oversized
   body/image, decompression bomb, redirect to private IP, DNS rebinding, stale
   search completion, repeated rapid search, addon disable/remove mid-request.
5. **Player:** known HTTPS MP4 and HLS, bad TLS/404, pause/seek/mute/volume/speed,
   audio/subtitle selection, buffering recovery, retry, resize, fullscreen enter/
   exit/close, EOF, app close, sleep/wake, and resume position.
6. **Packaging:** clean host with no mpv installation; bundled loader probe;
   dependency closure; code signature/notarization where applicable; license and
   corresponding-source artifacts; launch/play after moving the installed app.
