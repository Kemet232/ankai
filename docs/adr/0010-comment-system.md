# 10. Comment system (per-title discussion, votes, attribution)

Status: Proposed

Date: 2026-08-18

## Context

`PROGRESS.md`'s "Critical product audit," item 9, already names this class
of feature as needing real work before it ships: "Trust, safety, and legal
controls lag extension capability... ANKAI needs source attribution,
permission review, block/report paths, content-policy enforcement,
malicious-manifest handling... and a clear distinction between public
protocol compatibility and endorsement of particular content sources." The
human's own framing of this task (`PROGRESS.md`, "New candidates," item 9)
is explicit: a **real comment section** — per-title/release, visible to
everyone, with upvote/downvote and per-comment user attribution — "is not
scoped yet and is a genuinely architecture-level decision, not a quick UI
fix — same bar as ADR-0008/0009."

Two things this ADR is written against, both load-bearing:

1. **"Visible to everyone" rules out device-local state.** Every social
   surface ANKAI has today that stores user-authored content —
   `core::forum_posts` (community discussion posts), `core::communities`,
   `core::hangouts` — is explicitly local-only: each one's module doc
   comment says so plainly ("only exists in this device's own local
   encrypted DB until real P2P/community sync is built"). A comment thread
   that every user needs to see the same copy of cannot be built that way;
   it needs real shared server storage, the same category of thing
   `docs/adr/0008-identity-discovery-service.md` (directory) and
   `docs/adr/0009-account-system.md` (accounts) already built for their own
   domains.
2. **Nyaa is being removed entirely in a parallel track this session** (the
   human's explicit request, tracked separately from this one). Nyaa was
   the one place this codebase had *any* comment-adjacent feature before
   now: `core/src/nyaa.rs`'s "Nyaa anime upload comments"
   (`PROGRESS.md`, row "Nyaa anime upload comments") opened
   `https://nyaa.si/view/{id}` for a specific torrent upload's comment
   thread — explicitly **not** a title-wide thread, just a deep link to a
   third-party page for one release. That anchor (a `nyaa.si` upload id) is
   going away with the rest of that module. This ADR does not treat Nyaa's
   data model as precedent for "what a comment attaches to" — see "Which
   surface does a comment attach to" below, which works from the surfaces
   that actually survive Nyaa's removal.

Four sub-decisions are in scope, matching the human's own framing of the
open questions: **what a comment attaches to**, **storage/hosting shape**,
**auth/attribution model**, and **concrete abuse controls** (rate limits,
deletion/report path, moderation workflow).

**What this ADR does not do, unlike ADR-0008/0009:** it does not ship a
reference implementation. That was an explicit scope boundary set for this
track (docs-only, no Rust/Slint code) — not an oversight relative to
precedent. See "Reference implementation" below for what a future session
building one should read first.

## Decision

### Which surface does a comment attach to?

The real product surfaces that exist post-Nyaa-removal, per the task's own
framing, are: `core::stremio`'s addon-sourced `Meta`/`MetaPreview` (a title,
as returned by whichever addon/catalog surfaced it — `core/src/stremio.rs`),
`core::board`'s catalog entries (`BoardCatalogKey`/`BoardCatalogRef` —
`core/src/board.rs`, which identify *a catalog on an addon*, not a title),
and `core::anime`'s AniList-anchored watchlist (`AnimeSummary.id`, AniList's
own numeric media id — `core/src/anime.rs`).

The hard problem hiding in "attach a comment to a title" is that **ANKAI has
no single canonical title identity today.** The same show can arrive under
different ids depending on which addon returned it: Cinemeta (bundled,
IMDb-sourced) returns `Meta.id` values like `"tt1234567"`; the bundled Anime
Kitsu addon returns ids like `"kitsu:12345"`; `core::anime`'s AniList
integration uses its own unrelated numeric id space entirely, disjoint from
both. `core::board`'s `BoardCatalogKey` compounds this further by scoping
identity to `(addon_manifest_url, media_type, catalog_id)` — a catalog, not
a title, and addon-specific by construction (the same title reachable
through two different installed addons produces two different
`BoardCatalogKey`s that share no field).

**Rejected: attach comments to `BoardCatalogKey` or raw per-addon `Meta.id`
with no normalization.** This was the simplest option and the closest
analog to how Nyaa's release-scoped comments worked, but it fails the
product goal outright: the same title would get a different, unmerged
comment thread per addon a user happens to have installed, and threads
would break/orphan whenever a user uninstalls or reorders addons (since
`BoardCatalogKey` embeds the addon's exact manifest URL). Two people
discussing "the same show" through two different addons would never see
each other's comments. Rejected as not meeting "visible to everyone" in any
meaningful sense.

**Rejected: attach comments only to AniList ids (`core::anime`).** This is
the one place ANKAI already has a single canonical, addon-independent title
identity — but only for anime, and only for titles a user has actually
looked up through `core::anime`'s AniList integration. Movies and
non-anime series reached purely through Stremio catalogs (Cinemeta, or any
future non-anime addon) would have no comment surface at all under this
option. Given ANKAI's product scope is broader than anime, this would leave
most of the catalog without comments — rejected as too narrow, not as
technically wrong.

**Chosen: a canonical `ContentKey { namespace, external_id }`, built from
the id-namespace convention the Stremio addon ecosystem already uses, plus
AniList as an explicit third namespace.** This is not a new invention —
`core/src/stremio.rs`'s `Manifest.id_prefixes` (wire field `idPrefixes`,
already real protocol plumbing: `core/src/stremio.rs:683-684`, consumed by
`matches_prefix`/`Resource::matches` at `core/src/stremio.rs:1266-1273`) is
the Stremio ecosystem's own existing mechanism for addons to declare which
id namespace(s) they operate in. In practice this converges on a small,
recognizable set: `tt`-prefixed ids are the de facto IMDb namespace (used
by Cinemeta and most non-anime addons in the wider Stremio ecosystem, not
just ANKAI's bundled one), and `kitsu:`-prefixed ids are the Anime Kitsu
addon's own namespace (already visible in this repo's own test fixtures,
`core/src/stremio.rs:2161`, `2167`, `2219`). This ADR proposes normalizing:

- A `Meta`/`MetaPreview.id` beginning with `tt` → `ContentKey { namespace:
  "imdb", external_id: <the id, unprefixed> }`.
- A `Meta`/`MetaPreview.id` beginning with `kitsu:` → `ContentKey {
  namespace: "kitsu", external_id: <the numeric suffix> }`.
- Any other addon-specific id prefix ANKAI doesn't yet special-case → the
  raw `(addon-declared prefix or "unknown", id)` pair, unmerged with
  anything else — an explicit, narrow fallback, not a default everything
  routes through.
- `core::anime`'s AniList id → `ContentKey { namespace: "anilist",
  external_id: <the numeric id, as a string> }`.

The load-bearing property this buys: **two different addons that both use
the IMDb namespace convention for the same title land on the same
`ContentKey` without ANKAI doing any content matching at all** — the
identity comes for free from a convention the ecosystem already follows,
the same way it lets Stremio's own catalog-routing logic work today. This
is the cheapest correct thing available, not a guess.

**Named, not solved: cross-namespace unification.** A title that exists
*both* as an AniList entry (`anilist:16498`, say) *and* under an IMDb-space
id via some non-anime-oriented addon that also indexes it (`imdb:tt2560140`)
gets **two separate, unmerged comment threads** under this design — ANKAI
does no fuzzy title/year matching to recognize these as "the same show."
This is a real, likely-to-be-noticed gap for popular anime that also carry
IMDb entries. Solving it properly needs a title-matching/deduplication
service — a genuinely hard problem (name/year/studio fuzzy matching, false
positives that would merge wrong things, human review for edge cases) that
deserves its own research effort if it turns out to matter, not something
to bolt onto this ADR by guessing at a matching heuristic. Flagged as a real
open item in "Open questions" below, not hidden.

**Granularity: per-title, not per-episode, not per-release, for v1.**
Per-release doesn't survive Nyaa's removal (there is no ANKAI concept of "a
specific torrent/stream upload" with independent identity once Nyaa is
gone — `core::stremio::Stream`s are ephemeral, re-resolved per query, not
stored entities with stable ids). Per-episode (the MAL/AniList convention,
mainly to contain spoilers to "have you seen up to episode N") is a real,
recognizable product pattern this ADR consciously does not adopt for v1 —
seeded as an explicit open question below rather than decided, since it's a
genuine product tradeoff (thread fragmentation and spoiler safety vs. one
simpler, denser thread) this ADR shouldn't default silently.

### Storage/hosting shape

**Chosen: a new sibling crate, `server/comments/`, reusing
`docs/adr/0008-identity-discovery-service.md`'s already-researched stack
verbatim** — `axum` 0.8 over HTTP+JSON, `rusqlite` against a plain
(unencrypted) SQLite file, `tokio::task::spawn_blocking` for the sync-DB
pattern. No transport/storage re-litigation needed; that ADR's reasoning
(low-QPS-shaped payloads already `serde`-friendly, `rusqlite` already
vetted and in-tree, Postgres premature before any real deployed traffic)
applies unchanged here. See `docs/adr/0008` for the full research; not
repeated.

**Why unencrypted SQLite is still the right call, same reasoning as
`docs/adr/0008`/`0009`:** nothing this service stores is confidential —
comment bodies are, by the feature's own definition, public content the
author chose to post visibly. Encrypting a database holding no
confidential contents would be theater.

**Why a new crate, not folded into `server/directory` or `server/accounts`
— same reasoning shape `docs/adr/0009` already used to justify splitting
itself from `server/directory`:** comments have yet another distinct
durability/trust shape from either existing service. `server/directory`'s
rows are deliberately ephemeral (consumed-on-lookup, TTL'd). `server/
accounts`'s rows are durable authorization records with a revocation
lifecycle but a small, bounded write rate (device add/revoke is rare).
`server/comments` is durable **and** high-read (every viewer of any title
loads its thread) **and** carries a genuinely different abuse surface
(spam, harassment, brigading via mass-downvoting) that neither existing
service has to reason about at all. Splitting keeps each crate's trust
story statable in one sentence, the same bar `docs/adr/0009` set for
itself.

**One concrete difference from `docs/adr/0008`/`0009`'s traffic
assumption, worth flagging explicitly:** both existing services are
low-read, low-write, largely device-to-server or account-to-server
traffic. A public comment thread is read by *every viewer of a title*,
including users who never post anything — this is the first ANKAI service
whose read volume plausibly scales with the whole userbase's browsing
activity, not with account/device-management events. This ADR recommends
the reference implementation enable SQLite's WAL journal mode explicitly
(`PRAGMA journal_mode=WAL`) from the start, since WAL is what lets SQLite
serve concurrent readers without blocking on a writer — a small, concrete,
checkable change, not a re-litigation of the SQLite-vs-Postgres choice.
`server/comments` is also the most likely of the three server crates to hit
a real single-writer-process ceiling first, precisely because of this read
pattern — named here so a future session doesn't have to rediscover it.

### Auth/attribution model

Per-comment user attribution needs an identity that means something to
*other users*, not just to the posting device — the same reasoning
`docs/adr/0009-account-system.md`'s own context section already laid out
for why `core::identity`'s device-scoped `DeviceId`s can't deliver
"reliable... discovery" on their own. This ADR depends on `docs/adr/0009`
directly, which — as of this session — carries `Status: Accepted`
(`docs/adr/0009-account-system.md:3`). That removes what would otherwise be
this ADR's biggest open dependency; see "Consequences" for the real gap
that remains (ADR-0009's reference implementation is Accepted but **not
wired into `client`** yet).

**Chosen: comments and votes are authored by the account's current
*device* signature key, carrying that device's `DeviceRegistration` so any
verifier can resolve device → account entirely offline — no live call to
`server/accounts` required for signature verification.** Concretely, a
submitted comment carries:

```
Comment {
  content_key: ContentKey,           // see above
  body: String,
  device_registration: DeviceRegistration,  // core::account, ADR-0009
  account_public_key: Vec<u8>,       // so a verifier can check
                                      // derive_account_id(account_public_key)
                                      // == device_registration.account
                                      // without a network round-trip
  posted_at_unix: i64,
  device_signature: Vec<u8>,         // device key's signature over
                                      // (content_key, body, posted_at_unix)
}
```

Verification a `server/comments` instance (or any third party holding the
comment) can do **entirely offline**, no dependency on `server/accounts`
being reachable at read time:

1. `device_signature` verifies under
   `device_registration.device_public_key` (an ordinary Ed25519 check).
2. `core::account::verify_device_registration(&device_registration,
   &account_public_key)` — already-existing, already-tested code
   (`core/src/account.rs:309`) — confirms the registration itself is a
   genuine, unforged binding of that device key to that account.
3. `derive_account_id(&account_public_key) ==
   device_registration.account` — confirms the account public key supplied
   actually corresponds to the claimed `AccountId`, using the same
   already-existing self-certifying-id function
   (`core/src/account.rs:157`) `docs/adr/0009` built for exactly this
   purpose.

**Why the device key, not the account root key, signs every comment/vote
directly:** `docs/adr/0009`'s own design is explicit that the account root
private key is meant to be a comparatively high-value, infrequently-used
key — synced device-to-device only at pairing time, used mainly to
authorize new devices — not something every routine social action reaches
for. Reusing the already-established device-key-signs, `DeviceRegistration`
-proves-the-binding pattern keeps a comment's signing key exactly as
exposed as everyday P2P/directory traffic already makes device keys, and no
more. This mirrors `docs/adr/0009`'s own two-tier signing convention (a
naturally self-verifying signed object for the high-value case,
i.e. `DeviceRegistration` itself, vs. a generic signed envelope elsewhere)
rather than inventing a third pattern.

**Named, not solved: this proves authorization at the time the
`DeviceRegistration` was issued, not current revocation status.** A device
revoked via `server/accounts`' `POST .../devices/{device_id}/revoke`
(`server/accounts/src/app.rs:193`, `server/accounts/src/paths.rs`
`revoke_path`) can still submit comments carrying its old, validly-signed
`DeviceRegistration` — the offline verification above has no way to know
about a revocation it never fetched. Closing this properly needs
`server/comments` to consult `server/accounts`' device list at write time
(a real service-to-service dependency, or at minimum a periodically
refreshed local cache of revoked device ids) — not built or specified in
further detail here, flagged the same way `docs/adr/0008` flagged its own
"no replay-cache beyond the timestamp window" gap: acceptable to leave open
for a reference implementation nobody depends on yet, not for real
deployment. **`server/comments`' write paths (post comment, cast vote,
report, delete) should at minimum check revocation against `server/
accounts` at write time even if read-time verification stays fully
offline** — this is the concrete recommendation this ADR makes, left as a
"before Accepted" item.

**Votes reuse the same signature chain, but the vote *caster's* identity is
not necessarily shown to other users — only the aggregate count is.**
Concretely: `Vote { content_key, comment_id, account's DeviceRegistration +
account_public_key, value: 1 | -1, cast_at_unix, device_signature }`,
verified identically. The server enforces one live vote per `AccountId` per
comment (a new vote from the same account replaces, rather than adds to,
their prior vote on that comment — an upsert, not an append) and returns
only the summed total to readers. This matches the common product norm
(Reddit-style: vote counts are public, individual voters generally are
not) and reduces the social-graph-inference surface a fully public
per-voter record would create. Whether individual vote-caster identity
should ever be exposed is called out as an overridable default, not an
inevitability — see "Open questions."

**Reading a comment thread requires no account or signature at all** — a
plain, unauthenticated `GET`, matching `docs/adr/0008`'s precedent for
public key-package/address reads. Only posting, voting, deleting, and
reporting need the signed chain above.

### Concrete abuse controls

- **Content bounds.** Plain text only in v1 — no attachments, no rich
  formatting, no embedded media. A hard length cap on `body` (this ADR
  proposes reusing the existing bounded-string discipline already
  established in `core/src/stremio.rs` — e.g. its `MAX_SHORT_STRING`
  pattern at `core/src/stremio.rs:39` and `validate_string`/
  `validate_string_collection` helpers used throughout that module —
  rather than inventing a new validation convention; the exact cap number
  is not picked here, see "Open questions").
- **Rate limits.** Per-`AccountId` (not per-request, not per-IP — an IP-based
  limit would be trivially defeated by anyone behind CGNAT/a VPN, and
  ANKAI's whole identity model is already account-centric) limits on
  comment/vote/report submission rate, enforced server-side in
  `server/comments`, independent of any client-side throttling. Specific
  numeric thresholds are not picked here — they'd be arbitrary without real
  traffic data, the same caution `docs/adr/0008` implicitly avoided by
  picking TTLs tied to concrete protocol semantics (30-day `KeyPackage`
  expiry, 10-minute `EndpointAddr` staleness) rather than guessed abuse
  thresholds. Flagged as tunable, non-binding starting numbers for whoever
  builds the reference implementation to propose, not this ADR.
- **Deletion.** A comment's own author can soft-delete it (a signed
  `DELETE`, same signature chain as posting) — the row is tombstoned
  (content replaced with a `[deleted]` marker) rather than hard-removed, so
  thread structure/vote history stays intact and a moderator can still see
  what was posted. Symmetric to `docs/adr/0009`'s own choice to soft-delete
  (`revoked_at_unix`) rather than hard-delete device registrations, for the
  same audit-trail reasoning.
- **Report path.** Any signed-in account can file a `Report { comment_id,
  reason }` (also signature-authenticated, same chain — an unauthenticated
  report endpoint would itself be an abuse vector, e.g. mass-reporting to
  trigger automated takedowns). Reports accumulate; **no automated
  moderation action is specified** — a report changes nothing by itself,
  it only makes a comment visible to whatever moderation process exists.
- **Moderation authority is explicitly not decided by this ADR** — see
  "Open questions." This ADR provides the mechanism (reports land
  somewhere queryable) but not the policy (who acts on them, with what
  authority, under what appeals process). Building the mechanism without
  deciding the policy mirrors `docs/adr/0009`'s own choice to build
  device-revocation mechanics while leaving "who decides to revoke a
  compromised device" to product/support process, not the ADR.
- **Blocking is a client-side, device-local concern, not server
  policy.** "Hide comments from account X" is proposed as a local
  preference (same storage tier as `core::communities`'/`core::top8`'s
  existing local-only settings — no new server-side relationship graph,
  no new privacy exposure the way an *authenticated, server-visible* block
  list would create). This keeps blocking cheap to add without reopening
  `docs/adr/0008`'s already-named "no server-side relationship graph
  exists" gap.

## Reference implementation

**None in this session, deliberately** — this track's explicit scope is
docs-only (see this repo's session-24 track boundaries). This differs from
`docs/adr/0008`/`0009`'s pattern of pairing the ADR with real, tested code
in the same session; that pairing is *not* skipped out of laziness, it's an
explicit instruction for this specific track, stated up front so a future
reader doesn't mistake the absence of `server/comments/` for this ADR being
less thought-through than its predecessors.

What a future session building `server/comments/` should do first, in
order: (1) read this ADR in full, especially "Named, not solved" call-outs;
(2) read `docs/adr/0008-identity-discovery-service.md` and `server/
directory/` for the axum+SQLite+signed-request pattern this ADR reuses
verbatim; (3) read `docs/adr/0009-account-system.md` and `core/src/
account.rs` for `DeviceRegistration`/`verify_device_registration`/
`derive_account_id`, the exact primitives the auth design above is built
from; (4) resolve this ADR's "Open questions" with the human before writing
schema/endpoints that bake in an unreviewed answer to any of them.

## Consequences

**Positive:**

- Reuses every already-accepted architectural building block available —
  `docs/adr/0008`'s server stack, `docs/adr/0009`'s account/device
  identity chain, `core::stremio`'s existing `idPrefixes` convention — with
  no new cryptography, no new server technology decision, and no new
  identity primitive invented for this feature alone.
- The `ContentKey` design gets cross-addon comment-thread unification "for
  free" for any title reachable through IMDb-namespace or Kitsu-namespace
  addons, without ANKAI needing to build or operate any content-matching
  system.
- The offline-verifiable signature chain (device signature + embedded
  `DeviceRegistration` + account public key) means a comment's authenticity
  is independently checkable by anyone holding the account's public key —
  not just trusted because a particular server vouches for it — matching
  the self-certifying design ethos `docs/adr/0009` established.
- Vote-count-public/vote-caster-private by default reduces unnecessary
  social-graph exposure relative to a naively "everything signed is
  everything public" design.

**Negative / risks:**

- **This ADR has a real, current blocking dependency: `docs/adr/0009`'s
  reference implementation is Accepted but not wired into `client`.**
  `core::identity::load_or_create_device` still produces the old placeholder
  `AccountId` (a random opaque string, not an account-root-key-derived
  one — `core/src/identity.rs`'s own doc comment says so), and
  `core::account` is real, tested code that nothing in `client` calls yet.
  A comment system built exactly as designed here cannot actually let a
  user post anything authenticated until that wiring work lands — this ADR
  documents the target design, it does not shortcut around this
  dependency.
- **Cross-namespace title unification is a named, unsolved gap** (see
  "Which surface does a comment attach to" above) — expect real user
  confusion/complaints the first time a popular anime title's Cinemeta-side
  and AniList-side communities can't see each other's comments.
- **Revocation is not checked at offline-verification time** (see "Auth/
  attribution model" above) — a revoked device's old `DeviceRegistration`
  remains validly-signed forever from a pure-cryptography standpoint;
  closing this needs a live or cached cross-check against `server/
  accounts` that this ADR names but does not fully specify.
- **`server/comments` is the first ANKAI service whose read load scales
  with general browsing activity, not with account/device-management
  events** — the WAL-mode recommendation above mitigates but does not
  eliminate the risk that this is the first server crate to actually need
  Postgres before the other two do.
- **Moderation authority/process is entirely unspecified** — reports have
  somewhere to land, but nothing in this ADR says who acts on them. Without
  that answer, "abuse controls" here are necessary but not sufficient —
  matching item 9 of the critical-product-audit's own framing that this
  whole class of feature "needs... a clear distinction between public
  protocol compatibility and endorsement," which a report queue with no
  owner doesn't yet deliver.

**What would need to happen before this ADR could be marked `Accepted`:**

1. `docs/adr/0009`'s account system needs to actually be wired into
   `client` (real account creation/login flow, not just the reference
   `core::account` module) — without a real `DeviceRegistration` existing
   for real users, this ADR's entire auth design has nothing to sign with.
2. The human needs to answer this ADR's "Open questions" below — several
   are genuine product tradeoffs this document deliberately does not
   default on.
3. A real reference implementation (`server/comments/`, matching `server/
   directory`'s and `server/accounts`' testing bar: happy paths plus
   explicit rejection-path tests for forged signatures, revoked-device
   posting, cross-account vote/delete/report forgery) needs to exist and be
   reviewed, the same validation step ADR-0008/0009 required of themselves.
4. Same bar every prior ADR in this series has set for itself: human
   sign-off after actually reading and reacting to this document, not
   silent acceptance by inertia.

This ADR does not need a dedicated cohort-style spike — the transport/
storage/auth choices are all reused from already-validated precedent
(`docs/adr/0008`/`0009`); what's novel here (the `ContentKey` normalization
scheme, the abuse-control policy questions) is a design-review problem for
the human, not an unproven-technology problem a spike would resolve.

## Open questions (deliberately left for the human, not guessed)

1. **Ship order:** should comment posting wait for `docs/adr/0009`'s
   account system to be wired into `client` (the clean, designed-for path
   this ADR assumes), or is there appetite for an interim version
   attributed to the existing device-scoped `DeviceId`/TOFU model (the same
   posture `docs/adr/0008` itself adopted as an explicit bridge) with a
   named migration to account-based attribution once that wiring lands?
   This ADR recommends waiting (permanently, publicly attributed content
   inheriting TOFU's known impersonation weakness is a worse consequence
   than the same weakness in a private directory lookup — see
   `docs/adr/0008`'s "Still risky" section) but does not decide it, since
   it trades against real shipping-timeline pressure that's a product call.
2. **Moderation authority model:** who can act on filed reports — a small
   trusted-operator allowlist, an admin key analogous to but distinct from
   any user's account key, a delegated community-moderator role tied to
   `core::communities`, something else? Not decided here at all.
3. **Comment granularity:** whole-title threads (this ADR's v1 default) vs.
   per-episode threads for spoiler containment (the MAL/AniList norm) — a
   real product tradeoff between thread density and spoiler safety, not
   defaulted on.
4. **Cross-namespace title unification:** is user-visible thread
   fragmentation between (say) an AniList-anchored thread and an
   IMDb-namespace thread for "the same" title an acceptable v1 gap, or does
   it need its own follow-up research effort/ADR before shipping broadly?
5. **Vote-caster visibility:** this ADR defaults to aggregate-count-public,
   voter-identity-private (a design choice, not a technical necessity —
   the signature chain would support fully public per-voter records just
   as easily). Confirm or override.
6. **Numeric thresholds:** comment length cap, rate-limit thresholds
   (comments/votes/reports per account per unit time), and the report-count
   at which a comment might warrant a moderation review queue vs. just
   sitting in the report table — none picked here; this ADR intentionally
   leaves them as an implementation-time proposal rather than a guess made
   without real traffic data.

## Sources

- This repository's own `docs/adr/0008-identity-discovery-service.md`
  (transport/auth/storage precedent reused directly) and
  `docs/adr/0009-account-system.md` (account-root-key/`DeviceRegistration`/
  self-certifying-`AccountId` primitives this ADR's attribution model is
  built from — `core/src/account.rs`'s `derive_account_id`,
  `verify_device_registration`, `DeviceRegistration`).
- `PROGRESS.md`, "Critical product audit" item 9 (the trust/safety/
  attribution/abuse requirements this ADR is answering) and "New
  candidates from the human," item 9 (the task framing this ADR responds
  to directly).
- `core/src/stremio.rs`'s `Manifest.id_prefixes`/`idPrefixes` handling
  (`core/src/stremio.rs:683-684`, `1266-1273`) and its bundled-addon test
  fixtures (`core/src/stremio.rs:2161`, `2167`, `2219`) — the existing,
  real protocol convention this ADR's `ContentKey` namespace scheme
  normalizes against, not a new invention.
- `core/src/anime.rs`'s `AnimeSummary.id` (AniList's numeric media id,
  already the primary key of the local watchlist table) — the third
  `ContentKey` namespace.
- `core/src/board.rs`'s `BoardCatalogKey`/`BoardCatalogRef` — examined and
  explicitly rejected as a comment anchor (see "Which surface does a
  comment attach to").
- `core/src/forum_posts.rs`'s module doc comment — the explicit "device-
  local only" precedent this ADR's storage decision diverges from, and
  why.
- `core/src/nyaa.rs` (as it exists in this branch, pre-removal in the
  parallel Nyaa-removal track) — read only to confirm its comment feature
  was per-upload/deep-link, not a title-wide thread, and therefore not
  usable as this ADR's anchor design.
