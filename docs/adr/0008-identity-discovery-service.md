# 8. Identity/discovery service (directory)

Status: Proposed

Date: 2026-08-15

## Context

`docs/adr/0004-e2ee-stack.md` requires a directory: "each device publishes
`KeyPackage`s to a directory ahead of time, so a sender can initiate a
conversation before the recipient is ever online." `core/src/p2p.rs` needs
the same kind of service for a different purpose: learning a peer's current
`EndpointAddr` so a direct/relay-assisted QUIC connection can even be
attempted. `core/src/directory.rs` (session 9) built the client-side shape
of this as a `DirectoryService` trait plus an `InMemoryDirectory` reference
implementation, and stopped there deliberately — its own doc comment calls
picking real server technology "an undecided architectural question of the
same weight as ADRs 0002-0007." This ADR is that decision.

Four sub-decisions are in scope: **transport**, **auth**, **storage**, and
**staleness/expiry/consumption policy**. Hosting is not — ADR-0003 already
established that central ANKAI infrastructure exists for exactly this kind
of "thin cloud" service (identity/discovery/moderation/marketplace
metadata), and this service is squarely inside that boundary; nothing new
needs researching there.

What this service is *not* trying to solve: it does not carry any private
key material (a `KeyPackage` is, by MLS design, the public half of a
device's prekey — the whole point of publishing it to a semi-trusted
directory), and it does not carry message content. Per
`docs/threat-model.md`, the server operator is assumed hostile-curious for
message plaintext; this service never has plaintext to be curious about.

This ADR was researched by web search in August 2026, scoped narrowly
(unlike ADR-0003's five-technology survey) because the shape of the problem
— a low-QPS, small-payload, latency-insensitive CRUD service sitting next to
a Rust-native P2P/crypto stack that has already committed hard to Rust
end-to-end — leaves a much smaller field of genuinely live alternatives.

## Decision

### Transport: HTTP + JSON via `axum` 0.8, not gRPC/`tonic`

**Chosen: `axum`** (0.8, current stable is 0.8.9 as of ~June 2026), serving
plain JSON over HTTP/1.1+HTTP/2, with hand-rolled request signing headers
(see Auth below) rather than a generic auth middleware.

Why not gRPC (`tonic`), the natural alternative for a Rust-native RPC
service:

- gRPC's real advantages — bidirectional streaming, strict schema evolution
  via protobuf, HTTP/2 multiplexing efficiency — target problems this
  service doesn't have. Every RPC here is a small, one-shot request/response
  (publish one `KeyPackage`, look up a device's current set, publish/look up
  one `EndpointAddr`). ADR-0003 already gave the P2P layer (iroh/QUIC,
  gossip, unreliable datagrams) everything genuinely latency- or
  throughput-sensitive; this service is explicitly the "thin cloud" side,
  by design not on that hot path.
- Every payload this service moves (`KeyPackage`, `EndpointAddr`, `DeviceId`)
  is already `serde::Serialize`/`Deserialize` today — `identity.rs`'s
  `PublishedKeyPackage` already derives serde over a `KeyPackage` field, and
  `EndpointAddr` is already JSON-encoded by `core::messaging` for its
  paste-a-peer flow. Choosing gRPC would mean introducing a *second*
  schema/serialization system (protobuf `.proto` files, `prost`/`tonic-build`
  codegen) for data that already has a working, exercised serde path, purely
  to gain streaming/schema-evolution properties this service doesn't need
  yet.
- 2026 community discussion (see sources) converges on the same framing:
  Axum and Tonic solve different problems (HTTP vs. gRPC) and increasingly
  get *combined* in Rust projects — Axum 0.8's full Tower-middleware
  compatibility across HTTP and gRPC means adopting `tonic` later, for a
  genuinely streaming use case (e.g. a future server-push "your contact came
  online" notification), is a additive integration, not a rewrite. Nothing
  about picking `axum` now forecloses `tonic` later for a different traffic
  shape.
- `axum` is already Tokio-ecosystem-native (`tower`, `hyper`), matching
  every other async choice already made in this codebase (`tokio` is
  already a workspace dependency for `iroh`). No new async runtime, no new
  ecosystem to learn.
- A concrete, checkable fact from this repo's own `Cargo.lock`: `reqwest`
  0.13.4 and `rustls` 0.23.43 are *already* transitively present (pulled in
  by `iroh`/`iroh-relay`). Building the reference client on `reqwest` (see
  Implementation below) adds essentially zero new dependency-resolution
  surface — it promotes an already-resolved transitive dependency to a
  direct one, rather than introducing a new HTTP stack.

**Why not plain HTTP without axum (hand-rolled on `hyper` or raw TCP):**
axum's routing, extractors, and `IntoResponse`/tower-middleware plumbing are
exactly what a 4-endpoint JSON CRUD service needs and nothing more; hand-
rolling the same on bare `hyper` would just be reimplementing a thin slice
of axum, badly, for no benefit.

### Auth: request signing with the device's existing Ed25519 signature key — TOFU-pinned, not a full solution yet

The device already holds real key material for exactly this purpose:
`identity.rs`'s `Device.signature_key` (an `openmls_basic_credential::
SignatureKeyPair`, `SignatureScheme::ED25519`, backed by `ed25519-dalek`
under the hood — confirmed by reading `openmls_basic_credential`'s own
source, not assumed). Per README's non-negotiable ("no invented
cryptography"), this ADR reuses that key rather than adding a second key
type or a bespoke signing scheme.

**Chosen scheme:** every `publish_*` call carries three headers —
`x-ankai-device-pubkey` (hex-encoded raw 32-byte Ed25519 public key),
`x-ankai-timestamp` (Unix seconds), `x-ankai-signature` (hex-encoded 64-byte
signature) — over a canonical message:

```
sign( "{METHOD}\n{PATH}\n{DEVICE_ID}\n{TIMESTAMP}\n" ++ raw_request_body )
```

The server recomputes the identical canonical message from the request it
actually received and verifies it against the claimed public key using
`ed25519-dalek`'s `VerifyingKey::verify_strict` (that crate is already a
transitive dependency at 2.2.0 via `openmls_basic_credential`, so verifying
directly with it — rather than instantiating a full `OpenMlsProvider` just
to call its crypto trait's `verify_signature` — adds no new dependency and
is the leaner path for a server that has no MLS state of its own to
manage). A request outside a 5-minute timestamp skew window, or with a
non-verifying signature, is rejected (`401`) before touching storage.

**The open problem this does not fully solve:** signature validity proves
"whoever holds the private key for *this* public key sent this exact
request" — it does not, by itself, prove that public key is the one
genuinely associated with the claimed `DeviceId`. Nothing in the codebase
today creates that binding: `identity.rs`'s `DeviceId` is a random opaque
string, not derived from or co-signed by any higher-authority key.
`docs/threat-model.md`'s stated target ("device keys must be signed by the
account's root identity key; server never issues device trust on its own
authority") describes an account-root-key co-signing model that ADR-0004
named but that hasn't been built anywhere — `identity.rs`'s own doc comment
says `AccountId` is still "an honest placeholder... not derived from a real
account-root identity key."

Given that gap, this ADR adopts **trust-on-first-use (TOFU) pubkey pinning**
as the pragmatic bridge, not the final answer: the first validly-signed
publish call for a given `DeviceId` binds that public key to that id in
server storage; every subsequent call for the same `DeviceId` must verify
against the *same* pinned key, or is rejected (`403`, distinct from a bad-
signature `401` so a caller can tell "wrong key for this id entirely" apart
from "your signature didn't verify"). This closes the obvious hole (nobody
can silently overwrite another device's published material once that
device has published once) without inventing a PKI this codebase has no
foundation for yet. It reopens exactly once the account-root-key model
lands: at that point the server should require the account root key to
co-sign the *binding* (device key ↔ device id ↔ account), not just accept
whichever key shows up first.

**Also explicitly not solved here, called out because it's a real tension
with `docs/threat-model.md`:** *lookup* calls (`GET`) are left unauthenticated
in this design, matching `InMemoryDirectory`'s existing semantics (any
caller can look up any `DeviceId`'s `KeyPackage`s or current
`EndpointAddr`). For `KeyPackage`s this is by design — MLS's whole prekey
model assumes the directory is semi-trusted for availability, not
confidentiality, and a `KeyPackage` is public key material by construction.
For `EndpointAddr`, though, this is a real gap: the threat model lists "User
IP addresses (from untrusted peers, absent explicit consent)" as a
protected asset, and an unauthenticated `EndpointAddr` lookup lets anyone
who learns or guesses a `DeviceId` learn that device's current direct
network address — bypassing ADR-0003's whole privacy-mode / contact-trust
gating story, which assumes address disclosure is consent-gated. Fixing
this properly needs a server-side notion of "is the caller someone this
device has actually consented to be reachable by" — i.e., a real contact/
relationship graph — which doesn't exist server-side today (Communities is
local-only per session 8; there's no server-side social graph at all). This
ADR does not solve it; the reference implementation matches
`InMemoryDirectory`'s open-read semantics and flags this explicitly as a
pre-`Accepted` gap (see "What would need to happen" below), not something
quietly shipped as if it were fine.

### Storage: plain SQLite via `rusqlite`, not Postgres, not sqlx

**Chosen: `rusqlite`** (already a workspace dependency, currently used
client-side for the SQLCipher-encrypted local DB), talking to an
**unencrypted** SQLite file — no SQLCipher here, because nothing this
service stores is a secret. `KeyPackage`s are public prekey material by MLS
design; `EndpointAddr`s are network reachability hints, not credentials.
Encrypting a database that holds no confidential contents would be theater,
not defense.

Why not `sqlx` (the natural "modern async Rust SQL" alternative, and the
one this repo's own research already surfaced as SQLite-capable): `sqlx`'s
main advantage over `rusqlite` is native `async`/`.await` query execution
and multi-backend support (Postgres/MySQL/SQLite behind one API). Neither
matters here: this service's whole read/write surface is four simple
lookups/inserts against one small local file, and `rusqlite` is already a
vetted, license-clean, in-tree dependency (session 4's OpenMLS-storage work
already validated it). Adding `sqlx` would mean a second SQL crate in the
workspace, doing the same job `rusqlite` already does adequately, purely for
async ergonomics this service's request volume doesn't need. The reference
implementation wraps the (synchronous) `rusqlite::Connection` in a
`tokio::task::spawn_blocking` at each call site rather than blocking the
async runtime directly — the standard, boring fix for "sync I/O inside an
async server," not a reason to reach for a second database crate.

Why not Postgres: Postgres is the right call the day this service needs to
run as more than one instance (horizontal scaling, real HA), because SQLite
does not give concurrent writers across processes/machines. That day is not
today — there is no ANKAI Node/relay infrastructure deployed anywhere yet
(same reality ADR-0003's spikes are still waiting on), so provisioning and
operating a managed Postgres cluster for a service with zero real traffic
would be pure speculative infrastructure. This is exactly the kind of
"boring, real, sized-to-what-exists" choice `README.md`'s non-negotiables
and ADR-0003's own "central relay sizing... do not build a cost model that
assumes [things that haven't happened]" precedent call for. **This is the
single most likely thing to change before real deployment** — see
"Consequences" below.

Why not an ephemeral/Redis-like store even for `EndpointAddr` specifically
(considered, since addresses go stale fast and arguably don't need
durability at all): rejected for the reference implementation on
simplicity grounds — running a second storage system (Redis or similar) for
one of four endpoints, when SQLite already handles the TTL-at-read-time
policy below correctly, is added operational surface for a property (fast
eviction) that a `WHERE published_at > ?` clause already gets for free at
this scale. Worth revisiting only if `EndpointAddr` write volume ever
becomes the dominant cost driver — not expected at reference-implementation
traffic levels.

### Staleness / expiry / consumption

`core/src/directory.rs`'s doc comment poses this exactly: "a real directory
needs to decide whether a lookup consumes/removes what it returns, and what
happens once a device runs out." Decision, without changing the existing
`DirectoryService` trait's method signatures:

- **`KeyPackage`s are consumed by lookup.** A `key_packages()` call returns
  every currently-published, non-expired `KeyPackage` for a device *and*
  deletes exactly those rows from server storage in the same operation — a
  second immediate call returns empty. This is the closest fit to MLS's
  single-use-prekey intent that's expressible without changing the trait to
  return "one" instead of "every currently published" (the trait's own doc
  comment already frames the method as returning the whole currently-
  published set, not a single item scoped to one handshake) — it does mean
  a device that published five `KeyPackage`s and gets looked up once by one
  sender loses all five to that one sender, not one each to five different
  senders. That's a real limitation, explicitly not solved here: a proper
  one-time-prekey directory wants a "claim exactly one" operation, which
  would need a trait change (e.g. a new `claim_key_package` method
  alongside the existing bulk `key_packages`) that's out of scope for this
  ADR to impose unilaterally on a trait boundary session 9 already
  committed.
- **`KeyPackage`s also expire outright after 30 days**, even if never
  looked up, purely to bound storage growth from devices that published and
  never got contacted — enforced at read time (`WHERE published_at_unix >
  now - 30d`), not via a background sweep job. A real deployment likely
  still wants a periodic sweep to reclaim the on-disk space of never-
  consumed, expired rows (read-time filtering alone leaves dead rows on
  disk indefinitely) — noted as a gap, not built, consistent with keeping
  the reference implementation minimal.
- **`EndpointAddr` freshness is a 10-minute TTL, read-time enforced**, on
  top of the trait's existing "publish replaces" semantics (already
  implemented by `InMemoryDirectory`, carried over unchanged). Past the TTL,
  `endpoint_addr()` returns `None` — the trait's documented "never
  published" case and "published-but-stale" case are intentionally
  indistinguishable to callers, since a caller cannot act on a stale address
  any differently than a nonexistent one. A device that wants to stay
  discoverable needs to republish (heartbeat) inside that window; nothing
  in this codebase does that automatically yet (P2P scaffolding doesn't
  wire into the directory at all — see README's non-negotiables note in
  Consequences).
- **Revocation is explicitly not modeled**, matching `directory.rs`'s own
  callout. There is no higher-authority key to revoke *with* yet (see the
  Auth section's account-root-key gap) — this is blocked on the same
  unbuilt piece, not an independent gap this ADR could close on its own.

### Hosting

Same central ANKAI infrastructure tier ADR-0003 already established for
"thin cloud" services (identity/discovery/moderation/marketplace metadata).
No new research needed here; this service is a concrete instance of
capacity ADR-0003 already assumed would exist, not a new infrastructure
category.

## Consequences

**De-risked by this decision:**

- The transport, auth, and storage choices are all either already-present
  dependencies (`rusqlite`, transitively-present `reqwest`/`rustls`) or
  reuse of already-built key material (`identity.rs`'s device signature
  key) — this ADR adds comparatively little new dependency-resolution risk
  relative to ADR-0003's two-brand-new-stack decision.
- `axum`'s tower-middleware compatibility keeps the door open to adding
  `tonic` later for a genuinely streaming use case (e.g. server-push
  presence) without a rewrite, so choosing HTTP+JSON now isn't a one-way
  door away from gRPC.
- TOFU pubkey pinning, while not a complete auth story, closes the obvious
  hole (silent impersonation/overwrite of another device's published
  material) with no new cryptography and no dependency on unbuilt
  infrastructure — a real, if partial, improvement over the "no opinion at
  all" state `directory.rs` started from.

**Still risky / open — do not treat as settled:**

- **TOFU pubkey pinning is a bridge, not the destination.** It has a real
  race/griefing weakness: whichever client (legitimate or attacker) first
  publishes under a given `DeviceId` wins that binding permanently, with no
  recovery path modeled if a legitimate device loses its signing key before
  ever publishing (or a race is lost to an attacker guessing/observing a
  `DeviceId` before its real owner's first publish). This is acceptable
  only because nothing downstream depends on directory correctness yet
  (nothing is wired into `client`) — it would need to be replaced by
  account-root-key-co-signed device registration before any real user
  relies on this service. See threat-model's "forged device registration"
  threat, which this reference implementation does not fully close.
- **`EndpointAddr` lookups being unauthenticated is a genuine, undischarged
  tension with the threat model's IP-address protection goal**, not a minor
  detail — see the Auth section. This needs a server-side trust/relationship
  concept that doesn't exist yet before it can be called acceptable, let
  alone `Accepted`.
- **SQLite is a reference-scale choice.** The day this runs as more than a
  single process (real horizontal scaling, multi-region, actual HA), it
  needs to move to Postgres or an equivalent — flagged explicitly, not
  assumed away, matching ADR-0003's own precedent of naming a scaling cliff
  rather than silently building past it.
- **No replay-cache beyond the timestamp window.** A captured, validly-
  signed request can be replayed verbatim for up to 5 minutes before its
  timestamp ages out. A nonce/replay cache (e.g. reject a timestamp+
  signature pair already seen) would close this and isn't implemented in
  the reference server — acceptable for a reference implementation nobody
  depends on yet, not for real deployment.
- **`KeyPackage`-consumption-on-bulk-lookup (not per-handshake claim) is a
  real behavioral gap** from ideal one-time-prekey semantics, as described
  above — a device that published several prekeys can have all of them
  consumed by a single looker, defeating some of the point of publishing
  more than one.

**What would need to happen before this ADR could be marked `Accepted`:**

1. The reference implementation (this session's `server/directory/`) needs
   to actually run, under real concurrent load from more than a hand-built
   integration test, long enough to validate the SQLite/`spawn_blocking`
   storage path doesn't become a bottleneck at whatever traffic this
   service would realistically see pre-launch.
2. The TOFU pubkey-pinning gap needs either an explicit, deliberate "this is
   fine for launch because X" sign-off from whoever owns the threat model,
   or the account-root-key co-signing model needs to land first and this
   ADR needs to be revisited/amended to require it.
3. The unauthenticated-`EndpointAddr`-lookup tension with the threat model
   needs an explicit decision — either a real contact/relationship-scoped
   authorization model gets designed (a nontrivial new piece of work, likely
   its own ADR-adjacent decision), or someone explicitly accepts the current
   open-read behavior with eyes open and says so in this document.
4. Same bar ADR-0003 set for itself: human sign-off after actually reading
   and reacting to this document, not silent acceptance by inertia.

This ADR does **not** need its own NAT-traversal-style "run it across a
diverse cohort" spike — the transport/storage/auth choices here are
conventional enough (a JSON HTTP service backed by SQLite) that the open
questions are about correctness and trust-model completeness, not unproven
technology, and the reference implementation plus its integration test
(see `server/directory/`) is the validation step this ADR calls for before
`Accepted`.

## Sources

- [Rust Web Frameworks in 2026: Axum vs Actix Web vs Rocket vs Warp vs Salvo](https://aarambhdevhub.medium.com/rust-web-frameworks-in-2026-axum-vs-actix-web-vs-rocket-vs-warp-vs-salvo-which-one-should-you-2db3792c79a2)
- [Combining Axum, Hyper, Tonic, and Tower for hybrid web/gRPC apps](https://academy.fpblock.com/blog/axum-hyper-tonic-tower-part1/)
- [axum crate — docs.rs](https://docs.rs/axum/latest/axum/) (0.8.9, current stable as of this writing)
- [Rust ORMs in 2026: Diesel vs SQLx vs SeaORM vs Rusqlite](https://aarambhdevhub.medium.com/rust-orms-in-2026-diesel-vs-sqlx-vs-seaorm-vs-rusqlite-which-one-should-you-actually-use-706d0fe912f3)
- [`openmls_basic_credential::SignatureKeyPair` — docs.rs](https://docs.rs/openmls_basic_credential/latest/openmls_basic_credential/struct.SignatureKeyPair.html)
  and its source (confirms `ed25519-dalek` as the underlying ED25519
  implementation for `Signer::sign`)
- [`ed25519-dalek` — crates.io](https://crates.io/crates/ed25519-dalek) (dual
  MIT/Apache-2.0, already a transitive dependency of this workspace via
  `openmls_basic_credential`)
- This repository's own `Cargo.lock` (confirms `reqwest` 0.13.4 and
  `rustls` 0.23.43 are already transitively present via `iroh`/`iroh-relay`
  before this ADR's implementation adds anything)
