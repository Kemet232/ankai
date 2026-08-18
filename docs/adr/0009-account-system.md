# 9. Account system

Status: Accepted (2026-08-18, session 24)

Date: 2026-08-18

## Context

`core/src/identity.rs`'s own doc comment says it plainly: `AccountId` is "an
honest placeholder... a random opaque string, *not* derived from a real
account root identity key." There is no server-side account service, no
signup/login flow, and no way to recover an identity or add a second device
to the same account. `PROGRESS.md`'s "Critical product audit" names this as
the largest remaining architectural product dependency: device-scoped
IDs/usernames/friends (`server/directory`'s device-key TOFU pinning,
`core::friends`, `core::top8`) cannot deliver multi-device continuity,
account recovery, or reliable cross-device discovery, and — per
`docs/threat-model.md`'s "Identity & authentication" section, already
written before this ADR — "forged device registration" is a named threat
whose stated mitigation ("device keys must be signed by the account's root
identity key; server never issues device trust on its own authority") has
never actually been built.

This is the same shape of decision `docs/adr/0008-identity-discovery-service.md`
was: a piece `docs/adr/0004-e2ee-stack.md` (E2EE stack) already named and
partially designed ("Registration" model, recovery-key pattern) but
deliberately left unresolved as a distinct, later architectural decision.
Exactly like that ADR, this one stays **Proposed** — it proposes a design,
recommends one option per open question, and builds a real reference
implementation, but does not silently promote itself to `Accepted`. That is
a decision for a human to make after reading this document.

Four sub-decisions are in scope, matching what the task that produced this
ADR asked for explicitly:

1. **Auth mechanism** — how a human proves "I am this account" at all:
   password+email, magic-link/passwordless email, OAuth/social login, or an
   account-root-keypair model (the account *is* a keypair).
2. **Multi-device model** — how a second device gets authorized onto an
   existing account, and how MLS group membership updates when a device is
   added/removed.
3. **Hosting/backend tech** — reuse `docs/adr/0008-identity-discovery-service.md`'s
   already-accepted-pattern (axum + SQLite, ED25519-request-signing auth
   reusing device keys) unless there's a real reason it doesn't fit.
4. **Recovery** — what happens if a user loses every device, and which
   point in the zero-knowledge-vs-recoverable tradeoff space this product
   commits to.

What this ADR does **not** try to solve: the multi-device MLS-group-
membership fan-out (actually adding a newly authorized device to every
existing conversation's OpenMLS group, removing a revoked device from all
of them) is specified at a design level below but is explicitly **not**
implemented this session — see "Multi-device model" and "Consequences."
Building that requires a running, message-routing `client` with real
conversations to fan out into; this ADR's reference implementation is
server-side account/device bookkeeping only, the same "stops at a defensible
boundary, documents what's past it" discipline `docs/adr/0008` held itself
to for its own directory server.

## Decision

### Auth mechanism: the account root keypair model — not password, magic-link, or OAuth

**Chosen: an ANKAI account *is* an Ed25519 keypair** (the "account root
key"), generated entirely client-side, with a 24-word BIP39 recovery phrase
as the human-usable form of that same key material. No password, no email,
no third-party login. This is the same family of design as Signal's account
identity, Matrix's cross-signing master key, and — closest in spirit —
Session (Oxen)'s fully keypair-native, no-phone-number, no-email account
model. `docs/threat-model.md`'s "Identity & authentication" section already
named the mechanism this decision formalizes: "device keys must be signed
by the account's root identity key."

#### Options considered

| Option | Pros | Cons |
|---|---|---|
| **Password + email** | Familiar UX; industry-default; supports "forgot password" self-service | Directly in tension with `docs/adr/0004-e2ee-stack.md`'s "server never sees plaintext or keys": a password-reset flow is, by definition, a server-mediated way to regain control of an account's identity — even if the password itself never touches message plaintext, a compromised/abused reset flow lets an attacker mint new device authorizations for an account they don't own (the exact "forged device registration" threat `docs/threat-model.md` names). Requires operating password storage (hashing, breach-monitoring) and, in practice, an email-delivery pipeline neither of which this codebase has any of today. Ties identity to a real email address for a pseudonymous anime/gaming community where many users would rather not give one. |
| **Magic-link / passwordless email** | No password-storage risk; still familiar | Same core tension as password auth: whoever controls the recipient's email inbox can mint a new "login," which is again a server/email-provider-mediated path to account control that bypasses cryptographic device co-signing entirely. Requires reliable transactional email deliverability (a real operational dependency this project has never taken on). Same email-identity/privacy mismatch with the intended audience. |
| **OAuth / social login** (Discord, Google, Apple, etc.) | Zero password/email infra to build; Discord specifically has enormous overlap with ANKAI's anime/gaming audience; Apple requires offering "Sign in with Apple" if any third-party OAuth is offered on iOS, relevant given the iOS port raised in session 15 | Outsources the actual "who are you" decision to a third party ANKAI doesn't control — a banned/suspended/deleted Discord or Google account can strand an ANKAI account with no recourse, and a provider policy or outage becomes ANKAI's own outage. Still doesn't solve the actual cryptographic problem this ADR exists for: *something* has to mint the account-root key and co-sign devices, and OAuth only answers "let this person in," not "bind this device to this cryptographic identity" — the account-root-key work described below would still be needed *underneath* an OAuth login, making OAuth an additional layer of complexity and third-party trust rather than a replacement for it. |
| **Account-root-keypair + recovery phrase (chosen)** | Directly satisfies `docs/adr/0004-e2ee-stack.md`'s "server never sees plaintext or keys" — there is no server-mediated account-recovery path at all, so there is nothing for a compromised server or support process to abuse. Reuses this codebase's existing crypto-native pattern (`docs/adr/0008`'s ED25519-request-signing device keys) instead of introducing a second, unrelated auth paradigm. Fits the "thin cloud" philosophy and a privacy-conscious, pseudonymous audience — no email/phone/social-account requirement at all. Real, working precedent (Session/Oxen; Matrix's cross-signing keys, once loaded, work the same way). | No "forgot password" self-service — losing every device *and* the recovery phrase is unrecoverable by cryptographic design (see "Recovery" below; this is the explicit, accepted tradeoff, not an oversight). Recovery phrases are a UX category most consumer social-app users have never handled (unlike password reset, which almost everyone has done); real onboarding-education risk. Provides no "reset if the recovery phrase and every device are lost" safety net some users will expect from a mainstream app. |

#### Why the account-root-keypair model wins here specifically

This isn't a generic "keypairs are more secure" argument — the deciding
factor is that `docs/adr/0004-e2ee-stack.md` has *already* committed ANKAI
to a hard requirement ("server never sees plaintext or keys") that the
password/magic-link/OAuth options are all in real, structural tension with,
each for a different reason (see the table). The account-root-keypair
model is the only option in this list that doesn't need a carve-out or a
"but the reset flow is scoped narrowly enough that it's probably fine"
argument — there is simply no server-mediated path to account control to
reason about.

It also lets this ADR make one genuine, concrete improvement over
`docs/adr/0008-identity-discovery-service.md`'s own auth story, worth
calling out explicitly: `docs/adr/0008` had to adopt **TOFU pubkey
pinning** for `DeviceId`s — a first-write-wins race with a real,
acknowledged weakness (see that ADR's "Consequences" — "no recovery path
modeled if a legitimate device loses its signing key before ever
publishing"), because a `DeviceId` is an arbitrary opaque string with no
cryptographic relationship to any key. An account root key doesn't have
that problem: **`AccountId` is derived as a hash of the account root public
key** (`core::account::derive_account_id` — the first 16 bytes of its
BLAKE3 hash, hex-encoded), making it **self-certifying** — the same idea
behind Tor onion-service addresses and IPFS PeerIDs. Anyone who has an
account's public key can verify, entirely offline, that a claimed
`AccountId` genuinely corresponds to it. There is no "first write wins"
race for the account id itself, and no TOFU storage/pinning is needed
server-side to authenticate an account-level request — the reference
server (`server/accounts`) never pins or stores a "the first key we saw for
this id" record the way `server/directory` does; it just re-derives and
checks the hash on every request. This is a real, structural improvement
worth eventually back-porting: **once this ADR is `Accepted`, `docs/adr/0008`
should be revisited to require account-root-key-co-signed
`DeviceRegistration`s (defined below) instead of raw TOFU pinning** — that
ADR's own "Consequences" section already names this exact dependency
("it would need to be replaced by account-root-key-co-signed device
registration before any real user relies on this service").

#### Recovery phrase = account root key, not a separate secret

`docs/adr/0004-e2ee-stack.md` already named a "24-word BIP39-style
mnemonic" as the recovery secret for its client-side-encrypted backup
model. This ADR's design makes that the *same* secret as the account root
key, not a second one a user would have to separately generate and store:
the 32 bytes of entropy backing the account root key's Ed25519 signing key
(`SigningKey::from_bytes`) are the exact same 32 bytes BIP39 encodes into
the recovery phrase (`bip39::Mnemonic::from_entropy`/`.to_entropy()`).
Typing the phrase back in on a brand-new device deterministically
regenerates the identical account root keypair — see
`core::account::AccountRootKeyPair::generate`/`from_mnemonic`.

This is a deliberate simplification, not the standard BIP32/SLIP-0010
hierarchical-derivation path real cryptocurrency wallets use (that
machinery exists to derive *many* child keys from one seed via HMAC-SHA512
plus a chain code; ANKAI only ever needs one non-hierarchical account key,
so this ADR uses BIP39 purely for its mnemonic encode/decode of raw
entropy, not its seed-stretching/PBKDF2 machinery). **This composition
should be one of the specific things `docs/adr/0004-e2ee-stack.md`'s
required security audit signs off on before shipping** — see "Required
before shipping" below. If/when the recovery-phrase-encrypted backup blob
itself is built (per `docs/adr/0004`'s "Secondary, opt-in path"), reusing
one phrase for two purposes (signing identity + unwrapping a backup) safely
requires HKDF-style domain separation into distinct derived subkeys, not
literal reuse of the same 32 raw bytes for both a signature key and a
symmetric wrapping key — noted as a requirement here, not implemented,
since backup-blob encryption is `docs/adr/0004`'s job and out of this ADR's
scope (see "Recovery" below).

### Multi-device model

**Design (matching `docs/adr/0004-e2ee-stack.md`'s already-named
"Registration" model exactly):**

- A device generates its own Ed25519 signature keypair locally, exactly as
  `core::identity::load_or_create_device` already does today. This is
  unchanged by this ADR.
- To become part of an account, a device's public key gets bound into a
  **`DeviceRegistration`** (`core::account::DeviceRegistration`): `{
  account, device, device_public_key, signed_at_unix, signature }`, where
  `signature` is the account root key's signature over those fields
  (`core::account::sign_device_registration`/`verify_device_registration`).
  This is the concrete artifact that satisfies `docs/threat-model.md`'s
  "device keys must be signed by the account's root identity key" —
  independently verifiable by *anyone* who has the account's public key,
  not just by whichever server happens to be holding it at the time (see
  `server/accounts`'s `GET .../devices`, which returns both the account's
  public key and every current registration so a caller never has to trust
  the server's word for the binding).
- **First device (account creation):** generating the account root
  keypair *is* creating the account — there is no separate signup
  ceremony. The first device signs its own `DeviceRegistration`; the
  reference server's `POST /v1/accounts/{account_id}/devices` implicitly
  creates the account's server-side row the first time any validly-signed
  registration arrives for that id (see `server/accounts/src/app.rs::add_device`'s
  doc comment).
- **Adding a second device (already has an existing trusted device):** a
  QR-code/numeric pairing ceremony, per `docs/adr/0004-e2ee-stack.md`'s
  already-decided shape — ideally local over `docs/adr/0003-p2p-networking-stack.md`'s
  P2P layer when devices are colocated, otherwise the thin cloud relays
  only the small opaque pairing handshake. The new device generates its
  own device keypair locally (private key never leaves it); an *existing*
  trusted device co-signs a `DeviceRegistration` for it with the account
  root key.
- **Where does the account root private key live, for an already-trusted
  device to co-sign with?** Per `docs/adr/0004-e2ee-stack.md`'s "The
  account root identity key never leaves the originating device in
  plaintext" and its own "Syncing encrypted history to a new device"
  section, this ADR reuses that exact mechanism rather than inventing a
  new one: the account root private key is synced to each newly-trusted
  device the same way encrypted conversation history already is —
  primarily via direct trusted-device transfer over the P2P layer at
  pairing time, or via the same opt-in encrypted-backup-to-thin-cloud
  path, secondarily. It is not held only by a single "primary" device
  forever (a real single-point-of-failure risk this ADR deliberately
  avoids) — any device that has received it via a real pairing ceremony
  can co-sign future devices. It is stored at rest in each device's
  existing SQLCipher-encrypted local DB (`docs/adr/0004`'s local-encrypted-
  message-database layer), the same protection tier as everything else
  device-local.
- **Revocation:** symmetric to registration — `server/accounts`'s `POST
  .../devices/{device_id}/revoke`, authenticated by the account root key
  (see "Hosting/backend tech" below), soft-deletes (marks
  `revoked_at_unix`) rather than hard-deletes, so there's an audit trail
  and `GET .../devices` naturally excludes it from the active list. This
  directly closes the "Revocation is explicitly not modeled" gap
  `docs/adr/0008-identity-discovery-service.md` named and explicitly
  attributed to this exact missing piece ("There is no higher-authority
  key to revoke *with* yet").

**MLS group-membership updates on device add/remove — specified, not
implemented this session:** per `docs/adr/0004-e2ee-stack.md`, "a device is
just another group member... Adding a device to a 1:1 or group conversation
is an ordinary `Commit`." That's true at the protocol level, but it doesn't
solve the *orchestration* problem: when a new `DeviceRegistration` appears
for an account, something has to notice and, for every existing
conversation any of that account's other devices are already a member of,
issue an MLS `Add`-member `Commit` for the new device (symmetric `Remove`
for a revoked one). The design this ADR proposes: **each of an account's
other online devices, on observing a new/revoked `DeviceRegistration`
(e.g. via `server/accounts`'s open `GET .../devices` read, polled or
pushed), independently issues the corresponding `Commit` into every
conversation it's currently a member of.** This is genuinely complex real
work — it needs a running `client` with real conversation state to fan
into, delivery-service ordering per `docs/adr/0004`'s existing MLS
`Commit`-ordering requirement, and handling for the case where no other
device is online when a registration changes. **It is not built in this
session's reference implementation.** Honestly leaving this as specified-
but-unimplemented is preferable to faking it — see this ADR's own
"Consequences" and the task's explicit instruction not to build this if
genuinely complex.

### Hosting/backend tech: axum + SQLite, reusing `docs/adr/0008`'s pattern — as a sibling crate, not an extension

**Chosen: the same stack `docs/adr/0008-identity-discovery-service.md`
already accepted-in-pattern** — `axum` 0.8 over HTTP+JSON, `rusqlite`
against a plain (unencrypted — nothing stored is confidential, see below)
SQLite file, `tokio::task::spawn_blocking` for the sync-DB-in-async-server
pattern. No new technology decision needed; re-litigating transport/
storage choices `docs/adr/0008` already researched in depth (axum vs.
tonic, rusqlite vs. sqlx, SQLite vs. Postgres) would be pure waste — none
of that ADR's reasoning changes for this domain. See `docs/adr/0008` for
the full research; it isn't repeated here.

**New crate, `server/accounts/`, sibling to `server/directory/` — not an
extension of it.** Considered folding this into `server/directory` (it
already does device-key TOFU pinning, so there's some surface overlap) but
rejected: the two services have genuinely different data-durability and
trust shapes. `server/directory`'s `KeyPackage`/`EndpointAddr` rows are
deliberately ephemeral (consumed-on-lookup, 30-day/10-minute TTLs) and its
reads are open by design (public MLS prekey material). `server/accounts`'s
device registrations and recovery blobs are durable, authoritative records
with a real revocation lifecycle, and its recovery-blob endpoint is
deliberately *more* locked down (see below) than `docs/adr/0008`'s
precedent. Overloading one crate's auth model (TOFU-bridge-until-something-
better) with a second, stronger one (self-certifying, no TOFU at all) in
the same module would make both harder to reason about. Splitting them
keeps each crate's trust story simple enough to state in one sentence.

**Auth, two shapes depending on whether a naturally self-verifying signed
object exists for the operation** — a real, small improvement over
`docs/adr/0008`'s single envelope-signing scheme, made possible because
`DeviceRegistration` (unlike a bare `KeyPackage` publish) is itself already
a signed statement that needs to be independently verifiable later by third
parties, not just accepted once by the server:

- `POST /v1/accounts/{account_id}/devices` (add/refresh a device — also
  the only "create an account" operation there is, see above) is
  authorized entirely by the request body's embedded
  `DeviceRegistration.signature`, verified via
  `core::account::verify_device_registration` plus a
  `REGISTRATION_SKEW_SECS` (300s) freshness check on its own
  `signed_at_unix`. No separate envelope signature — it would just be a
  second signature proving the same fact the embedded one already proves.
- `POST /v1/accounts/{account_id}/devices/{device_id}/revoke` and
  `PUT`/`GET /v1/accounts/{account_id}/recovery-blob` have no such
  naturally-signed object ("revoke device X" isn't independently
  meaningful to a third party), so they use a generic signed-HTTP-envelope
  scheme (`server/accounts/src/auth.rs`) — the same canonical-message
  shape (`method\npath\nid\ntimestamp\n` + body) `docs/adr/0008`'s device-
  key request signing already established, reused deliberately rather than
  inventing a second convention in the same codebase. The one real
  difference: the claimed public key is checked against the URL's
  `account_id` by **content-derivation**
  (`core::account::derive_account_id`), not TOFU pinning — see "Auth
  mechanism" above.
- `GET /v1/accounts/{account_id}/devices` (list an account's current
  devices) stays **unauthenticated**, matching `docs/adr/0008`'s open-read
  precedent for public key material — any peer needs to be able to
  discover an account's current devices to add a newly-joined one to
  existing MLS groups (see "Multi-device model" above).
- `GET /v1/accounts/{account_id}/recovery-blob` is **authenticated**
  (unlike `docs/adr/0008`'s fully-open `KeyPackage`/`EndpointAddr` reads)
  — a deliberately stricter choice than that ADR's precedent, because a
  recovery blob is a materially higher-value target (ciphertext an
  attacker could work against indefinitely offline) than a `KeyPackage`.
  This is possible without breaking the actual recovery UX because the
  account root key is derivable from the recovery phrase *locally, before
  any network call* — by the time a recovering user's new device needs to
  fetch the blob, it already holds the exact key needed to sign the
  request.

### Recovery: recovery-phrase-only, no server-assisted path — v1, matching `docs/adr/0004-e2ee-stack.md`'s already-stated default

**Chosen: if a user loses every device and their recovery phrase, the
account is unrecoverable by design.** This is not a gap — it's the explicit
tradeoff `docs/adr/0004-e2ee-stack.md`'s "Account recovery" section already
committed to ("If the recovery key is lost, the backup is unrecoverable by
design — this is the explicit tradeoff of not letting the server help,
matching Signal's own stated model") and this ADR inherits it rather than
reopening it. Recovery consists of:

1. **Primary path**: trusted-device-transfer (see "Multi-device model"
   above) — most users never touch the recovery phrase except as a
   break-glass path, exactly as `docs/adr/0004` already specified.
2. **Break-glass path**: type the 24-word recovery phrase into a fresh
   client install. This deterministically reconstructs the account root
   keypair (`AccountRootKeyPair::from_mnemonic`) — no server involved in
   that step at all — and the freshly-derived key can then immediately
   sign requests against `server/accounts` (register this device as a new
   `DeviceRegistration`, fetch the opt-in encrypted `recovery-blob` if one
   was ever uploaded) to re-establish presence and, if a backup exists,
   recover conversation history. **This ADR's reference implementation
   builds the opaque-blob storage half of this** (`server/accounts`'s
   `recovery-blob` endpoints, real and tested) **but not the blob's actual
   contents/encryption** — what goes into the blob (an MLS state snapshot?
   just the account root key, since devices already sync history
   peer-to-peer?) and how it gets encrypted client-side is
   `docs/adr/0004-e2ee-stack.md`'s design to make, already gestured at
   there, genuinely not this ADR's to redo.
3. **No fallback beyond that.** No server-side password reset, no
   "recovery contact" scheme, no support-ticket override. Consistent with
   `docs/adr/0004`'s "Hard requirement carried forward: no server-side key
   escrow... and no scheme where ANKAI infrastructure alone... can
   reconstitute a user's private keys."

**Honest cost, named explicitly, matching the task's request not to paper
over it:** real users *will* lose accounts permanently under this model —
losing a phone, never writing down 24 words, and having no other device
online is not a hypothetical. `docs/adr/0004-e2ee-stack.md`'s own
"Consequences" already flagged this as "a real support/UX cost the product
team must own explicitly," and this ADR doesn't reduce that cost, it just
confirms the design commits to it rather than quietly building a
server-recoverable escape hatch that would violate the E2EE guarantee.
`docs/adr/0004` names a possible future v2 (a Signal-SVR2-style low-entropy-
PIN recovery backed by a rate-limited oblivious service/enclave) as
explicitly deferred, not required for launch, and not required here either
— it would need its own audit and its own ADR-level decision if the product
team decides the loss rate is unacceptable.

## Reference implementation

- **`core/src/account.rs`** (new module): real, tested cryptography —
  `AccountRootKeyPair::generate`/`from_mnemonic`, `derive_account_id`
  (self-certifying `AccountId`), `DeviceRegistration` plus
  `sign_device_registration`/`verify_device_registration`. **Not wired
  into `core::identity::load_or_create_device` or `client`** — matching
  `docs/adr/0008`'s own precedent (`core::directory`'s trait existed a full
  session before that ADR was written, and another session before
  anything used it), this stays real, tested, additive machinery backing a
  `Proposed` ADR. `identity::load_or_create_device` is unchanged; `client`
  still produces exactly the same placeholder `AccountId` it always has.
- **`server/accounts/`** (new crate): a real `axum`+SQLite server
  (`src/app.rs`, `src/db.rs`, `src/auth.rs`, `src/paths.rs`) implementing
  `POST`/`GET /v1/accounts/{account_id}/devices`, `POST
  .../devices/{device_id}/revoke`, and `PUT`/`GET .../recovery-blob`, plus
  a real network client (`src/client.rs::AccountsClient`) mirroring
  `server/directory`'s `HttpDirectoryClient` shape. **Not wired into
  `client`** — same reference-implementation-behind-a-`Proposed`-ADR
  posture as `server/directory`.
- **Tests**: unit tests in `core::account` (mnemonic round-trip, tampered/
  forged-registration rejection, self-certification, secret-redacted
  `Debug` output) and throughout `server/accounts` (`auth`/`db` unit
  tests), plus `server/accounts/tests/roundtrip.rs` — a real `axum::serve`
  instance on a real local TCP port, exercised entirely over genuine HTTP
  via `AccountsClient`, matching `server/directory`'s integration-test bar:
  happy paths (add/list/revoke a device, recovery-blob round-trip,
  **a full break-glass recovery scenario that reconstructs a real
  `AccountsClient` from a recovery phrase on a "new device" and fetches the
  same blob back**) alongside genuine rejection paths (unauthenticated
  add/revoke/recovery-blob calls, a forged `DeviceRegistration` signed by
  the wrong key, a different account attempting to steal an already-
  registered device id, an attacker with a *real but different* account
  key attempting to forge a revoke against someone else's account path,
  and an oversized recovery blob).
- **Not implemented, honestly**: the multi-device MLS-group-membership
  fan-out (see "Multi-device model" above); the recovery-blob's actual
  contents/client-side encryption (per `docs/adr/0004-e2ee-stack.md`'s
  existing design, not this ADR's job); the QR-code/numeric device-pairing
  ceremony's actual UI/protocol; the account-root-key-transfer-at-pairing-
  time mechanism (specified, reuses `docs/adr/0004`'s existing encrypted-
  history-sync design, not separately implemented).

## Consequences

**Positive:**

- Directly compatible with `docs/adr/0004-e2ee-stack.md`'s "server never
  sees plaintext or keys" — no server-mediated account-recovery path
  exists to be a structural weak point, unlike password/magic-link/OAuth.
- `AccountId` becomes self-certifying (a real improvement over
  `docs/adr/0008-identity-discovery-service.md`'s TOFU-pinned `DeviceId`,
  and a concrete, named path to eventually strengthen that ADR too).
- Reuses this codebase's existing crypto-native, ED25519-request-signing
  pattern end-to-end (device keys per `docs/adr/0008`, now account keys
  too) instead of introducing an unrelated auth paradigm (password
  hashing, email delivery, OAuth provider integration) this project has
  never needed before.
- No new infrastructure category: `server/accounts` reuses exactly
  `docs/adr/0008`'s already-researched hosting decision.
- The recovery phrase and the account identity key are unified into one
  secret a user has to protect, not two.

**Negative / risks:**

- No account-recovery safety net beyond the recovery phrase — real,
  permanent account loss will happen for real users who lose every device
  and the phrase. This is an accepted, named tradeoff (see "Recovery"
  above), not an oversight, but it is a real product/support cost.
- Recovery phrases are an unfamiliar UX pattern for a mainstream social-app
  audience; onboarding/education risk not addressed by this ADR (a product/
  UX design question, not an architectural one).
- The BIP39-entropy-as-raw-Ed25519-seed composition is a deliberate
  simplification versus standard BIP32/SLIP-0010 wallet derivation and
  needs explicit audit sign-off (see "Auth mechanism" above and "Required
  before shipping" below) — not yet reviewed by anyone outside this ADR.
- The multi-device MLS-group-membership fan-out is specified but
  unimplemented — until it's built, adding a device to an account does
  *not* automatically give that device access to existing conversations;
  that gap is real and would need to be closed before this ADR's
  multi-device story is actually usable end-to-end.
- `server/accounts`'s recovery-blob endpoint only stores/serves opaque
  bytes; the actual backup format and its client-side encryption
  (`docs/adr/0004-e2ee-stack.md`'s job) don't exist yet, so "recover my
  conversation history from the cloud" isn't actually possible yet even
  though the storage plumbing for it is real and tested.
- Two separate signing conventions now exist in `server/accounts`
  (embedded-object signature for `/devices`, generic envelope for
  everything else) — simpler than it sounds once the rationale is
  understood (see "Hosting/backend tech" above), but it is a second
  pattern next to `docs/adr/0008`'s single envelope scheme, worth keeping
  in mind for anyone extending either server.

**Required before shipping to real users (gate, not optional):**

1. This ADR's account-root-key/`DeviceRegistration` design must go through
   the **same independent professional security audit**
   `docs/adr/0004-e2ee-stack.md` already requires before shipping E2EE to
   real users — that audit's scope explicitly grows to include this ADR's
   composition (BIP39 entropy used directly as an Ed25519 seed, the
   self-certifying `AccountId` derivation, `DeviceRegistration` signing/
   verification, and — once built — the HKDF domain separation between the
   account root key's signing use and any future recovery-blob-wrapping
   use). This ADR does not add a *second* audit requirement; it is one
   more thing the existing required audit must cover.
2. The multi-device MLS-group-membership fan-out (see "Multi-device
   model") must be designed in detail and implemented before "add a
   device" is a real, usable feature — until then, this ADR's device-
   registration/revocation machinery proves *authorization* works, not
   that a newly authorized device can actually read existing
   conversations.
3. `docs/adr/0004-e2ee-stack.md`'s recovery-blob format/encryption must be
   built before the recovery-blob storage this ADR provides is useful for
   anything beyond storing arbitrary bytes.
4. Once this ADR is `Accepted`, `docs/adr/0008-identity-discovery-service.md`
   should be revisited/amended to require account-root-key-co-signed
   `DeviceRegistration`s in place of its current TOFU pubkey pinning — that
   ADR already names this exact dependency as blocking its own path to
   `Accepted`.
5. Same bar every prior ADR in this series has set for itself: human
   sign-off after actually reading and reacting to this document, not
   silent acceptance by inertia.

This ADR does not need a dedicated cohort-style spike the way
`docs/adr/0003-p2p-networking-stack.md`'s NAT-traversal/WebRTC work did —
the transport/storage/auth choices here are conventional (the same
JSON-HTTP-over-SQLite shape `docs/adr/0008` already validated), and the
genuinely novel piece (the account-root-keypair/self-certifying-id/
recovery-phrase composition) is a cryptographic-design question the
required security audit above is the correct validation mechanism for, not
a network/hardware spike.

## Sources

- This repository's own `docs/adr/0004-e2ee-stack.md` ("Multi-device,"
  "Account recovery" sections — the "Registration" model and recovery-key
  pattern this ADR formalizes were already named there) and
  `docs/adr/0008-identity-discovery-service.md` (the transport/auth/
  storage precedent this ADR reuses, and the TOFU-pinning gap this ADR
  proposes eventually closing).
- `docs/threat-model.md`, "Identity & authentication" section (the
  "forged device registration... device keys must be signed by the
  account's root identity key" requirement this ADR implements).
- [`bip39` crate — docs.rs](https://docs.rs/bip39/latest/bip39/struct.Mnemonic.html)
  (2.2.2, CC0-1.0 licensed — already on `deny.toml`'s allow-list, confirmed
  via its own crates.io listing) — `Mnemonic::from_entropy`/`.to_entropy()`/
  `FromStr` are the only API surface used; its seed-stretching (`to_seed`)
  and this crate's own BIP32-adjacent tooling are deliberately not used —
  see "Auth mechanism" above.
- [`ed25519-dalek` — crates.io](https://crates.io/crates/ed25519-dalek)
  (already a direct dependency of `server/directory` per `docs/adr/0008`'s
  research, and a transitive dependency of `ankai-core` via
  `openmls_basic_credential` — this ADR promotes it to a direct `ankai-core`
  dependency at the same already-resolved major version, adding no new
  major version to the dependency graph).
- BLAKE3 — already named and license-cleared in `docs/adr/0004-e2ee-stack.md`'s
  licensing summary ("MIT/Apache-2.0/CC0 triple-licensed") for safety-number
  fingerprints; this ADR is its first actual use in this codebase
  (`core::account::derive_account_id`), promoted from an already-resolved
  transitive dependency to a direct one (see root `Cargo.toml`).
- Prior art consulted for the account-root-keypair design: Session
  (Oxen)'s fully keypair-native account model (no phone/email at all);
  Matrix's cross-signing master key (loaded onto a new device via a
  recovery passphrase or trusted-device verification, functionally the
  same shape as this ADR's `AccountRootKeyPair`); Signal's Secure Backups
  recovery-key pattern (already the direct basis of
  `docs/adr/0004-e2ee-stack.md`'s recovery design, reused here for the
  account root key too).
