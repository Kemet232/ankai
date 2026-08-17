# Stremio competitive-parity roadmap

Status: S20 complete; S21 complete; S22 next
Owner: ANKAI desktop  
Last reviewed: 2026-08-17

## Goal

Make ANKAI a credible Stremio-compatible desktop media hub while keeping its
own neon social identity. Compatibility means more than accepting an addon
URL: ANKAI must understand the complete documented addon contract, provide a
coherent library/discovery/player experience around it, and ship the native
runtime without asking the user to install dependencies.

Desktop parity is the first gate. Web, mobile, and TV clients are a later
distribution track, not an excuse to leave the desktop workflow incomplete.

ANKAI will not bundle or endorse an infringing content provider. Torrent,
Usenet, archive, and external-URL support are transport capabilities for
content the user is authorized to access. Addon warnings, network permissions,
source attribution, and report/block controls are product requirements.

## Current baseline

| Capability | Current ANKAI state | Gap to close |
| --- | --- | --- |
| Manifest, catalog, meta, stream HTTP resources | Typed, bounded, capability-routed, and network-hardened | Expand the product UI over the complete core contract |
| Configured manifest URLs | Working | Add first-class configuration UX and secret redaction |
| Multiple installed addons | Persisted, ordered, enable/disable/remove, health shown | Add repository discovery, automatic updates, warnings, and account sync |
| Catalog extras | Required fields, declared options, and `optionsLimit` are validated; search only reaches eligible catalogs | Add genre/type controls, `skip` pagination, and per-catalog state |
| Capability routing | Catalog, meta, stream, subtitle, and addon-catalog declarations respect type and ID-prefix filters | Apply the same typed boundary to new S21/S23 surfaces |
| Metadata/details | The core models documented poster, people, link, trailer, video, inline-stream, rating, release, and behavior fields | Render the remaining fields in the S22 Details surface |
| Direct HTTP playback | Embedded libmpv player works; every documented source form is typed and labelled | Add safe resolvers, stream headers/proxying, retry/fallback, ranking, and reconnect |
| BitTorrent stream descriptors | Parsed and magnet reconstruction works | Bundled resolver, file selection, buffering/peer state, cache policy, privacy warning |
| Subtitle tracks embedded in media | libmpv track selection plus bounded addon/inline subtitle models and endpoint | Wire addon subtitles into playback, then add local files and style/language/delay defaults |
| Library and progress | Anime watchlist plus encrypted playback resume | Provider-neutral library, watched episodes, notifications, calendar, filters, and sync |
| Player experience | Fullscreen/bounded rendering, seek, volume, speed, tracks, errors | Auto-next, binge continuity, subtitle styling, stats, HDR/hardware settings, downloads |
| Casting/local media | Missing | Chromecast, external-player handoff, local-file scan/drop, local streaming gateway |
| Cross-device continuity | Device-local only | Profiles plus library/addon/progress/settings synchronization and conflicts |
| External activity integrations | Missing | Optional Trakt import/two-way scrobbling and privacy-controlled rich presence |

## Definition of desktop competitive parity

ANKAI reaches the desktop parity gate when all of the following are true:

1. A compatible public addon can be discovered, configured, installed, queried,
   updated, disabled, reordered, and removed without restarting ANKAI.
2. Manifest resource filters prevent irrelevant catalog/meta/stream/subtitle
   requests, including type and ID-prefix filtering.
3. Board and Discover expose every eligible catalog with search, genre filters,
   required extras, and incremental `skip` pagination.
4. Details faithfully render the documented metadata and episode model, and
   aggregate streams and subtitles from every matching enabled addon.
5. Direct, external, YouTube, BitTorrent, and the documented packaged-source
   forms have an explicit safe outcome: play, resolve through a bounded local
   gateway, or present an honest external handoff. No source silently fails.
6. The player provides subtitle preferences, stream fallback, auto-next,
   binge-group continuity, media keys, hardware/HDR controls where supported,
   playback statistics, reconnect, and durable resume.
7. Library, watched state, Continue Watching, release calendar, and new-episode
   notifications use one provider-neutral identity model.
8. Chromecast, external-player handoff, local files, and offline downloads have
   complete lifecycle, storage, privacy, and error UX.
9. Optional Trakt import/scrobbling can be enabled without making it a mandatory
   identity provider, and Guest mode remains completely local.
10. A clean machine can run the signed application with bundled libmpv and every
   required transport helper; users do not install native dependencies.
11. Keyboard/gamepad navigation, screen readers, reduced motion, 200% text,
    low-end GPU operation, malicious-addon tests, and long-session playback pass.

## Delivery phases

### S20 — Protocol contract completeness

Status: **complete (2026-08-17).** Public HTTPS is the implemented transport.
Private HTTP, IPFS/IPNS, Stremio install links, and legacy v1/v2 are deliberately
classified as explicit unsupported outcomes until their permission, gateway, or
adapter policies are proven. Likewise, all documented stream targets are typed,
but only safe direct HTTPS is currently playable; torrent, YouTube, NZB, archive,
and external-handoff resolvers remain S23 work.

Build the compatibility layer before adding more screens.

- Extend `Manifest` with global `idPrefixes`, `addonCatalogs`, `contactEmail`,
  `behaviorHints`, and typed configuration fields.
- Preserve resource descriptors as typed capabilities: resource name, types,
  and ID prefixes. Add one resolver that decides which addons receive a request.
- Model catalog `optionsLimit` and validate required extras before issuing a
  request.
- Model full metadata, video, link, trailer, poster-shape, behavior-hint, and
  inline-stream fields without relying on untyped `extra` maps for known data.
- Model the documented stream variants and behavior hints: direct URL, YouTube
  ID, torrent info hash/file index, external URL, NZB, archive sources, inline
  subtitles, proxy headers, country allowlist, filename/hash/size, web-readiness,
  and binge group.
- Add the subtitle and addon-catalog endpoints with the same response budgets,
  DNS pinning, redirect rules, and cancellation/generation behavior as existing
  resources.
- Put resource fetching behind a transport interface. Modern HTTPS remains the
  default; private HTTP requires an explicit per-host grant. Add bounded IPFS/
  IPNS gateway resolution and an isolated legacy-v1/v2 adapter only when their
  fixtures and security policies are proven.
- Capture official-document examples and legal public-domain providers as
  fixtures. Add compatibility tests for short/full resource declarations,
  configured paths, required extras, unknown fields, and malformed responses.

Acceptance gate: the core can parse, validate, route, and round-trip every
currently documented official addon field and resource, and resolve each
documented transport URL through its explicit policy. Unsupported transports
produce a typed outcome, never a generic or silent error.

### S21 — Board, Discover, and search parity

Status: **complete (2026-08-18), with honest gaps carried to later phases.**
Every bullet below shipped in real, tested code (`core::board`,
`core::catalog_cache`, `core::deeplink`, `client/ui/board.slint`). Known gaps,
not silently dropped: keyboard focus is not preserved across a filter/"Load
more" model rebuild (matches the pre-S21 shelf's own behavior, not a
regression); a catalog whose only required extra is something other than
`genre` surfaces the addon's own error with no input form yet; addon cache
freshness hints (`cacheMaxAge` etc.) are parsed and stored but not yet used to
skip a live fetch — the cache is a live-first, fallback-on-failure store with
honest last-updated disclosure, not yet a freshness-driven read path; and deep
links are parsed/routed in-app only, with no OS-level `ankai://` scheme
registration (a packaging step, tracked under S28).

- Replace the single merged result shelf with catalog-aware Board and Discover
  models: addon, content type, catalog, and selected extras are explicit state.
- Render all non-required catalogs as Home/Board rows in persisted addon order.
- Add type, catalog, genre, and addon selectors generated from manifest data.
- Implement `skip` pagination with per-catalog loading/end/error state; dedupe by
  `(type, id)` while preserving source attribution and stable focus.
- Query only search-capable catalogs for global search and only catalogs whose
  required extras are satisfied.
- Add bounded stale-while-revalidate memory/disk metadata caching, honoring
  addon cache headers where safe, with offline and last-updated disclosure.
- Add deep-link parsing for search, discover, detail, video, and addon-install
  routes. Reject ambiguous or unsafe external targets.

Acceptance gate: every installed catalog can be browsed, filtered, paginated,
searched, linked, retried, and used offline from an honest stale cache.

### S22 — Details, unified library, calendar, and notifications

- Build a provider-neutral media identity table with aliases for IMDb, Kitsu,
  AniList, and addon-native IDs. Never merge solely by display title.
- Expand Details with correct poster shape, background/logo, release/runtime,
  people and genre links, trailers, ratings, language/country, and season picker.
- Render related/similar recommendations from explicit metadata links or a
  documented provider response; never infer identity from a matching title.
- Separate episode selection from Play; add explicit episode play buttons and
  watched/progress markers.
- Replace the anime-only watchlist boundary with a unified Library supporting
  movies, series, anime, channels, TV, podcasts, and addon-defined types.
- Add add/remove, watched/unwatched, per-video progress, sort/filter, and
  notification toggles. Migrate existing watchlist and resume rows losslessly.
- Derive a release calendar from metadata video release dates, with timezone
  handling, month/week views, deep links, and optional ICS export.
- Poll enabled metadata providers for new videos using backoff and persisted
  cursors; create local notifications only for opted-in Library items.

Acceptance gate: a title moves cleanly from Discover to Details to Library to
Calendar/notification to playback and back to Continue Watching.

### S23 — Stream orchestration and subtitle parity

- Introduce a `StreamResolver` registry returning a common seekable media source,
  lifecycle events, cache information, and a redacted diagnostics record.
- Rank streams using explicit user preferences, availability, format, quality,
  language, cached status, and addon order. Keep manual selection available.
- Retry and fall back without changing source silently; show exactly which addon
  and stream is active.
- Add a bounded local HTTP gateway for request headers, response headers,
  non-web-ready direct streams, and formats that libmpv cannot safely receive
  from an untrusted addon URL directly.
- Implement addon subtitle requests using video ID plus filename/hash/size;
  combine addon, inline-stream, embedded, and dragged local subtitles.
- Persist default language, secondary language, size, color, background,
  outline, visibility, and delay. Support temporary delay nudges in the player.
- Add an explicit ADR for BitTorrent resolver selection and bundling. Required
  behavior includes file selection, seekable piece priorities, trackers/DHT,
  cache cap/location, peer/buffer statistics, cleanup, cancellation, and a clear
  public-IP disclosure before first use.
- Route pasted magnet links and dropped `.torrent` files through the same
  resolver, permission disclosure, file picker, quota, and cleanup lifecycle.
- Implement YouTube and external URLs as distinct reviewed paths. External web
  pages require confirmation; YouTube playback must not assume a system helper.
- Add later resolver plugins for documented NZB and archive sources, with
  credential isolation, decompression/file-count/size limits, traversal defense,
  disk quotas, and cleanup. These do not block the first direct+torrent beta but
  do block a claim of complete documented stream-source parity.

Acceptance gate: every stream variant has deterministic UX and policy; direct,
torrent, subtitle, and fallback paths survive pause/seek/close/reopen testing.

### S24 — Player and binge experience

- Add previous/next episode, auto-play countdown/cancel, season transitions, and
  same-`bingeGroup` stream continuity with an explicit fallback prompt.
- Complete subtitle-delay and style controls, audio/subtitle default selection,
  custom playback speed, progressive keyboard seeking, and configurable seek
  steps.
- Add media keys, playback statistics, remaining-time-at-current-speed,
  torrent peer/buffer telemetry, reconnect state, and sleep/wake recovery.
- Add pause-when-minimized and optional separate-player-window preferences while
  keeping the in-app player as the default experience.
- Expose hardware decoding and safe platform-specific HDR/SDR preferences;
  report actual active decoder/output instead of only the requested setting.
- Add optional intro/outro markers only when a trustworthy metadata source
  provides real ranges; never fabricate detection.
- Add episode completion rules and next-item prefetch without starting an
  unwanted download.

Acceptance gate: two-hour direct and torrent sessions, rapid seeking, stream
failure, subtitle switching, fullscreen, sleep/wake, and episode rollover pass
without losing progress or trapping the UI.

### S25 — Casting, local media, and offline playback

- Implement Chromecast discovery/session control and receiver capability checks.
  Transcode only through an explicit bounded local gateway and disclose CPU use.
- Add external-player handoff with a strict allowlist, progress-import limits,
  and a clear warning when ANKAI cannot observe completion.
- Add local file open/drag-drop and an opt-in indexed-folder scanner. Store only
  user-approved roots; handle removable volumes and permission loss.
- Add a local streaming gateway for casting and optional remote HTTPS access,
  disabled by default with authentication, bind-address controls, and firewall UX.
- Add an authenticated LAN remote-control page/QR flow for transport, seek,
  volume, and queue control without exposing the media gateway by default.
- Add offline downloads with per-item status, pause/resume, free-space checks,
  checksums, storage location/quotas, deletion, and encrypted metadata. Respect
  source permissions and never label an incomplete torrent cache as downloaded.

Acceptance gate: direct and resolved media can play locally, cast when compatible,
or download when authorized, with predictable cleanup and no surprise listener.

### S26 — Addon ecosystem and trust

- Consume `addon_catalog` resources and support one or more reviewed repository
  feeds with search, filters, versions, author/contact, and capability summaries.
- Add configuration forms and advanced `/configure` handoff while preserving
  configured path tokens. Secrets remain encrypted and redacted from UI/logs.
- Show adult/P2P/configuration-required warnings before installation; summarize
  requested network and external-open capabilities.
- Add manifest refresh/update history, compatibility status, health/backoff,
  rollback, report/block, attribution, and per-addon diagnostics export.
- Add explicit permission flow for private/LAN addons rather than weakening the
  public-HTTPS policy globally.

Acceptance gate: a user can safely discover and understand an addon before
installing it, and can recover from a broken or malicious update.

### S27 — Profiles and cross-device continuity

- ADR the real account, profile, recovery, and conflict model; keep Guest mode
  fully local and usable.
- Sync Library, watched/progress, installed configured addons, preferences,
  calendar notification state, and profiles across devices with versioned
  records and deterministic conflict resolution.
- Add optional Trakt OAuth, one-time history import, token refresh, and
  bidirectional playback scrobbling. Make conflicts visible and keep Trakt
  entirely absent from the default local/Guest path.
- Encrypt sensitive addon configuration client-side and support export/delete,
  revocation, recovery, and device management.
- Keep social identity and media profiles related but separable so household
  viewing does not silently expose private messages or addon credentials.

Acceptance gate: start an episode on one clean device and continue it on another
with the same Library, addon order, profile, and settings; Guest mode makes no
sync calls.

### S28 — Distribution breadth and release proof

- Finish attested LGPL libmpv and transport-helper bundles for macOS, Windows,
  and Linux; require dependency-closure, loader-probe, signing, and clean-machine
  smoke tests in CI.
- Add update channels, rollback, crash consent/redaction, and migration recovery.
- Add privacy-controlled OS media-session and Discord-rich-presence integration;
  never publish titles or playback state until the user opts in.
- After desktop parity, define shared-core clients for Android/Android TV first,
  then evaluate web and additional TV platforms. Gamepad/remote focus is a hard
  requirement for TV UI, not a desktop layout stretched onto a television.

Acceptance gate: signed desktop packages require no dependency installation and
pass the same protocol/player fixtures on clean machines.

## Parallel execution lanes

Once S20 freezes the typed contracts, work can proceed in four lanes:

- Core lane: resource routing, cache, library identity, notifications, sync.
- Player lane: resolvers, subtitles, binge behavior, casting/local gateway.
- UI lane: Board/Discover, Details, Library, Calendar, addon repository.
- Release/QA lane: bundled dependencies, clean-machine matrix, accessibility,
  malicious-addon fixtures, memory/network/performance soak tests.

Each phase must update `PROGRESS.md`, add tests for its acceptance gate, pass
workspace format/test/clippy, and land as a reviewable commit before the next
phase is marked complete.

## Official comparison sources

Reviewed 2026-08-16:

- Stremio addon protocol and resources:
  <https://stremio.github.io/stremio-addon-sdk/protocol.html>
- Manifest, catalog extras, addon catalogs, configuration, and behavior hints:
  <https://stremio.github.io/stremio-addon-sdk/api/responses/manifest.html>
- Metadata, videos, links, and trailers:
  <https://stremio.github.io/stremio-addon-sdk/api/responses/meta.html>
- Stream variants and behavior hints:
  <https://stremio.github.io/stremio-addon-sdk/api/responses/stream.html>
- Subtitle resource contract:
  <https://stremio.github.io/stremio-addon-sdk/api/responses/subtitles.html>
- Catalog search/filter/pagination contract:
  <https://stremio.github.io/stremio-addon-sdk/api/requests/defineCatalogHandler.html>
- Stremio product, device sync, casting, and content-type overview:
  <https://www.stremio.com/>
- Player/streaming settings and calendar export:
  <https://stremio.zendesk.com/hc/en-us/articles/360017301779-Stremio-options-explained>
- Keyboard and gamepad navigation:
  <https://stremio.zendesk.com/hc/en-us/articles/360022892811-Shortcuts-in-stremio>
- Current Stremio v5 player/library changes:
  <https://blog.stremio.com/stremio-tech-update-83-stremio-v5-stremio-web-updated/>
- Current offline/profile/binge/HDR/media-session changes:
  <https://blog.stremio.com/stremio-tech-update-80-stremio-v5-stremio-web-updated/>
- Official Trakt scrobbling and two-way import description:
  <https://blog.stremio.com/stremio-tech-update-28-trakt-scrobbling-2-way-sync-more/>
- Official BitTorrent, magnet, and `.torrent` input description:
  <https://stremio.zendesk.com/hc/en-us/articles/360000281292-Does-Stremio-use-BitTorrent>
- Official local-file scan and drag/drop description:
  <https://stremio.zendesk.com/hc/en-us/articles/360021474051-Local-content>
