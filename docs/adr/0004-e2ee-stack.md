# 4. End-to-end encryption stack

Status: Accepted
Date: 2026-08-13

## Context

ANKAI's private communications (1:1 DMs, community/group private channels,
voice/video calls, file transfers) must be end-to-end encrypted so that
ANKAI's own servers — assumed **hostile-curious** per `docs/threat-model.md`
— never see plaintext, private keys, or recovery secrets. Requirements
carried over from the product spec and threat model:

- Authenticated key exchange + ratcheting E2EE for 1:1 conversations
  (forward secrecy, post-compromise security).
- A proven group-encryption protocol for group chats / community private
  channels (MLS suggested in the product spec — verified below, not assumed).
- E2EE for voice/video calls where technically appropriate, with the
  WebRTC-transport-vs-messaging-layer boundary made explicit.
- Encrypted file transfer.
- A local encrypted message database that integrates with ANKAI's chosen
  local persistence, SQLite (see `docs/threat-model.md`'s Local Data
  section).
- Multi-device: registration, verification/safety-number fingerprints, key
  rotation, syncing encrypted history to new devices.
- Account recovery that never gives the server plaintext or private keys.
- Abuse reporting: a user can voluntarily decrypt and submit *specific*
  offending messages as evidence without weakening E2EE for anyone else
  (this exact tension is already called out in `docs/threat-model.md`).

**Hard constraint:** no custom cryptographic primitives. Every choice below
is an established, reviewed protocol or library, not a bespoke construction.

**Licensing constraint:** ANKAI's product includes a closed, proprietary
marketplace backend (creator 90/10 split, payments, entitlements). A
network-copyleft dependency (AGPL/GPL) pulled into anything that talks to
or is combined with that backend would create real legal exposure. This ADR
treats permissive licensing (MIT/Apache-2.0/BSD-style) as a hard filter for
any crypto dependency, flagged explicitly for the `docs/adr/*` license
manifest ("PLAN 51").

## Decision

Unify on **MLS (RFC 9420) via OpenMLS**, used for both 1:1 conversations and
group/community channels, rather than running two separate crypto stacks
(Signal-Protocol-style double ratchet for 1:1 + MLS for groups). Layer
voice/video and file-transfer encryption on top of the same MLS group state
rather than inventing separate key-exchange ceremonies for each.

| Use case | Choice | Protocol | License |
|---|---|---|---|
| 1:1 DMs | OpenMLS, 2-member group | MLS / RFC 9420 | MIT |
| Group chats & community private channels | OpenMLS, N-member group | MLS / RFC 9420 | MIT |
| Voice/video calls (1:1 and group) | SFrame, keyed from MLS `exporter_secret`, layered on WebRTC/DTLS-SRTP | RFC 9605 (SFrame) | N/A (spec; our implementation over RustCrypto AEAD) |
| File transfer | Per-file AEAD key wrapped via MLS `exporter_secret` | XChaCha20-Poly1305 (RustCrypto) | MIT/Apache-2.0 |
| Local message DB | SQLCipher-encrypted SQLite (fallback: SQLite3 Multiple Ciphers) | AES-256/ChaCha20-Poly1305 at rest | BSD-style / MIT |
| Recovery | Client-side-encrypted backup blob, recovery-key wrapped | Matrix-SSSS / Signal-Secure-Backups pattern | N/A (our composition) |

### Why not libsignal (Signal Protocol) for 1:1?

`libsignal` (Signal's official Rust/multi-language library implementing
X3DH/PQXDH + Double Ratchet) is the obvious first choice for 1:1 messaging
and is exactly the kind of audited, battle-tested library this ADR should
prefer — except for licensing: **`libsignal` is AGPL-3.0-only**, and Signal
states "use outside of Signal is unsupported." No public commercial
dual-licensing program for third parties was found as of this research
(2026). AGPL's network-copyleft clause would obligate ANKAI to release
complete corresponding source of any program combined with it to every user
who interacts with it over a network — directly in tension with a closed
marketplace backend. Rejected as a **dependency** (its design is still
useful prior art).

### Why not vodozemac (Matrix's Olm/Megolm reimplementation)?

`vodozemac` is a clean, Apache-2.0-licensed, pure-Rust reimplementation of
Olm (double ratchet, X3DH-like) and Megolm (sender-key group ratchet), and
had a clean 2022 Least Authority audit with no significant findings —
license-wise it would otherwise be a strong pick. However, independent
researcher Soatok publicly disclosed a **high-severity** Diffie-Hellman
key-validation flaw in February 2026 (identity-element / all-zero public-key
acceptance, affecting both Olm and Megolm — it makes the resulting shared
secret predictable). Matrix's security team publicly disputed the practical
severity, and as of this writing no fix has landed in the public changelog.
Given the hard "no unresolved/disputed crypto" bar for this ADR, vodozemac
is **not used**. This is a maturity/disclosure judgment, not a license
judgment — revisit if/when this is independently resolved and re-audited.

### Why MLS instead of Signal-Protocol-family for everything, including 1:1?

- **Security parity**: MLS's TreeKEM gives the same forward secrecy and
  post-compromise security as a double ratchet, for groups of any size, in
  O(log n) instead of O(n) per membership change.
- **One codebase, one audit surface** instead of maintaining a double
  ratchet implementation and a separate MLS implementation with different
  failure modes.
- **Multi-device falls out for free**: in MLS, a device is just another
  group member. Adding a device to a 1:1 or group conversation is an
  ordinary `Commit` — no bespoke device-linking protocol needed.
- **Real-world precedent for 1:1-over-MLS specifically**: Wire migrated
  its 1:1 conversations (not just groups) from its own Signal-Protocol-derived
  Proteus to MLS via its Rust `core-crypto` library. GSMA RCS Universal
  Profile 3.0 (2025) standardized cross-platform E2EE for RCS — including
  1:1 texting between Android and iOS — on MLS; Apple and Google shipped
  this in beta in May 2026. This is no longer a niche pattern.
- **OpenMLS is now independently audited**: Security Research Labs
  completed a security assurance assessment (final report v1.1, March 2026)
  of OpenMLS. 8 issues found, 1 rated High; 7 of 8 are fixed and shipped in
  openmls 7.3/8.1, with 1 Low-severity issue still being addressed as of
  this research. OpenMLS is MIT-licensed, feature-complete against RFC 9420
  with two small documented exceptions (not itemized in public sources
  found), has 55+ contributors and 1800+ commits, and is used in 181+
  downstream repositories.
- **Crypto backend**: use `openmls_rust_crypto` (the RustCrypto-based
  provider), MIT/Apache-2.0 dual-licensed — avoid any backend with less
  permissive terms.

**Tradeoff accepted**: MLS requires a **Delivery Service** to totally order
`Commit`s per group/conversation — including 1:1 DMs, which a pure
asynchronous double ratchet would not need. ANKAI's thin-cloud relay
already exists for offline delivery/push in the fat-client architecture; it
plays the Delivery Service role here too, sequencing opaque ciphertext only,
never plaintext. This needs explicit design attention in ADR-0003 (P2P
networking) so conversation-state advancement doesn't become a hard
single-point-of-failure on the central service being reachable. Async
first-contact (starting a DM with an offline user) works the same way
Signal's prekeys do: each device publishes `KeyPackage`s (MLS's equivalent
of prekeys) to a directory ahead of time, so a sender can initiate a
conversation before the recipient is ever online.

### Group chats / community private channels

Same OpenMLS groups as above — this is exactly what MLS was designed for,
and TreeKEM's O(log n) membership-change cost is what makes even large
community channels (hundreds+ of members) tractable, unlike naive pairwise
Signal-style fan-out (removing one member from a 500-person Signal-style
group would mean ~499 individual key-distribution messages).

### Voice/video calls — the WebRTC-layer boundary, made explicit

WebRTC's built-in **DTLS-SRTP** already encrypts the RTP media hop-by-hop
between each peer and whatever it's directly connected to (a peer, or an
SFU relay). That's necessary but **not** end-to-end for a group call
through an SFU: the SFU itself can see plaintext media unless a further
layer is added.

- **1:1 calls**: DTLS-SRTP alone is sufficient against network attackers.
  For defense against a compromised/coerced relay/TURN server carrying the
  call, additionally layer **SFrame (RFC 9605)** via WebRTC insertable
  streams, with the frame-encryption key derived from the already-
  authenticated MLS 2-member group's `exporter_secret`. This reuses
  identity/authentication already established by the messaging layer — no
  separate key-exchange ceremony, conceptually the same idea as how Signal
  binds RingRTC's call keys to an existing Signal Protocol session, just
  sourced from MLS instead.
- **Group calls (Hangouts / community voice)**: same SFrame layer, keyed
  from the group's `exporter_secret` for the current epoch, rotated on
  membership change so a member who leaves mid-call cannot decrypt
  subsequent media (RFC 9420's own epoch semantics handle this; SFrame's
  per-sender key-ID scheme, described in `draft-barnes-sframe-mls`, is
  exactly the IETF-documented way to wire MLS exporter secrets into SFrame).
  The SFU only ever forwards ciphertext — matching `docs/threat-model.md`'s
  existing "ANKAI Node" trust boundary (relays trusted for availability,
  never for confidentiality).
- **RingRTC** (Signal's Rust SFU/calling library, with a similar frame-
  encryption design) is AGPL-3.0-only — rejected as a dependency for the
  same reason as libsignal, but is useful public reference architecture for
  how a Rust SFU implements this pattern.
- **Flag**: there is no mature, audited, off-the-shelf Rust crate that does
  "SFrame + WebRTC insertable streams" turnkey today. ANKAI will need to
  implement the SFrame framing itself (a straightforward application of
  AEAD framing per RFC 9605, not a new primitive) over whichever WebRTC
  media stack ADR-0002/0003 select. **This integration is a dedicated audit
  line item before group voice/video E2EE ships.**

### Encrypted file transfer

Each file gets a random per-file content key, encrypted with
XChaCha20-Poly1305 (RustCrypto `chacha20poly1305` crate, MIT/Apache-2.0
dual). The content key is itself wrapped using the conversation's current
MLS `exporter_secret`, so only current group members can unwrap it — the
same pattern used for call keys above. The resulting ciphertext blob moves
as opaque bytes over ANKAI's P2P layer (ADR-0003, preferred, thin-cloud
philosophy) or is relayed/cached by an ANKAI Node if the recipient is
offline; blobs are content-addressed by ciphertext hash for integrity and
dedup, the same pattern Matrix uses for encrypted attachments and Signal
uses for attachment encryption. Large files are chunked and streamed, with
each chunk independently AEAD-framed (bounded memory, resumable transfer,
tamper-evident per chunk, no new primitive).

### Local encrypted message database (SQLite integration)

- **Primary**: **SQLCipher** via `rusqlite`'s `bundled-sqlcipher` feature.
  SQLCipher Community Edition is BSD-style-licensed with an attribution
  requirement — permissive, fine for a closed commercial product. (A paid
  SQLCipher Commercial Edition exists for official support/perf add-ons;
  not required.) SQLCipher is the most battle-tested encrypted-SQLite
  option in the ecosystem (used by Signal Desktop itself, among others).
- **Fallback**: **SQLite3 Multiple Ciphers** (MIT, github.com/utelle) if
  SQLCipher's OpenSSL vendoring (the `bundled-sqlcipher-vendored-openssl`
  feature) ever becomes a license-manifest concern — it's fully MIT with a
  pluggable cipher scheme (AES-256, ChaCha20-Poly1305, others), letting us
  standardize on the same AEAD family used elsewhere in this stack.
- **Key management**: the DB encryption key is a device-local secret,
  generated on first run, stored via OS-level secure storage (macOS
  Keychain / Windows Credential Manager / Linux Secret Service, via the
  `keyring` crate, MIT/Apache-2.0) — never derived from or escrowed with
  the server, per `docs/threat-model.md`'s Local Data section. It may
  additionally be gated behind an OS-level unlock (passphrase/biometric).
- **This is a distinct layer from MLS**: SQLCipher protects data *at rest*
  against device theft/loss; MLS protects data *in transit* and against
  server/relay compromise. Both are required — decrypted MLS plaintext is
  what lands in the SQLCipher-encrypted tables.
- OpenMLS needs its own persistent keystore for group/ratchet state between
  epochs. Back OpenMLS's storage trait with the same SQLCipher-encrypted
  SQLite file rather than standing up a second storage engine — one
  encrypted file, one key, simpler operationally and for backup/recovery.

### Multi-device

- A device is an MLS "client": its own signature keypair, its own
  published `KeyPackage`s. Joining an existing 1:1 or group conversation
  from a new device is an ordinary MLS `Commit` ("add member") — no
  separate device-linking protocol needed, which is the structural payoff
  of unifying on MLS.
- **Registration**: matches `docs/threat-model.md`'s identity threat notes
  — new-device trust is granted by an *existing* trusted device
  co-signing/approving it (QR or numeric pairing ceremony, ideally local
  over ADR-0003's P2P layer when devices are colocated, otherwise via the
  thin-cloud purely as a relay for the small opaque pairing handshake). The
  account root identity key never leaves the originating device in
  plaintext; the server never issues device trust on its own authority.
- **Verification / safety numbers**: derive a per-contact fingerprint as a
  stable hash (BLAKE3 — MIT/Apache-2.0/CC0 triple-licensed) of the sorted
  concatenation of a contact's currently-active device signature public
  keys. Render as Signal-style numeric groups plus a scannable QR code;
  recompute and surface a "safety number changed" warning whenever a
  device is added or removed — mirroring Signal's safety numbers and
  Matrix's cross-signing/SAS verification.
- **Key rotation**: MLS's per-`Commit` tree-secret update already rotates
  keys continuously (that's post-compromise security by construction).
  Additionally schedule periodic no-op "self update" commits (policy: e.g.
  every N days of conversation inactivity) to bound exposure even in quiet
  conversations — standard MLS operational hygiene, not a new mechanism.
- **Syncing encrypted history to a new device** — never plaintext through
  the server:
  - *Primary path*: direct trusted-device transfer. The new device and an
    already-verified existing device authenticate to each other (via the
    pairing ceremony above) and stream re-encrypted history directly over
    the P2P layer (ADR-0003) or a local link. Zero server trust required.
  - *Secondary, opt-in path*: encrypted history backup to the thin-cloud.
    The client encrypts a backup blob client-side under a recovery key
    (below) before upload; the server stores only inert ciphertext. This
    mirrors Signal Secure Backups (rolled out to everyone February 2026: a
    64-character recovery key generated on-device, server never sees it or
    the plaintext, no server-side password reset) and Matrix's SSSS
    (Secure Secret Storage and Sharing).

### Account recovery (no server access to plaintext or private keys)

- **v1 model**: recovery-key pattern (Matrix SSSS / Signal Secure Backups).
  On setup, the client generates a high-entropy recovery key (24-word
  BIP39-style mnemonic or an equivalent 64-character string) and uses it —
  via Argon2id (the `argon2` crate, MIT/Apache-2.0 dual, RustCrypto) — to
  derive a wrapping key. The account root identity key and the latest MLS
  state snapshot are encrypted client-side under that wrapping key
  (XChaCha20-Poly1305) before the ciphertext is uploaded to the thin-cloud
  as opaque storage. The server never sees the recovery key or the
  plaintext. If the recovery key is lost, the backup is unrecoverable by
  design — this is the explicit tradeoff of not letting the server help,
  matching Signal's own stated model.
- **Preferred UX**: trusted-device transfer (above) so most users never
  touch the recovery key except as a break-glass "lost every device" path.
- **Explicitly deferred (v2/stretch)**: a Signal-SVR2-style low-entropy-PIN
  recovery backed by a rate-limited oblivious service (SGX/AWS-Nitro-
  enclave-based OPRF) would give a much friendlier UX (short PIN instead
  of 24 words) but requires operating trusted-execution-environment
  infrastructure and its own independent audit. Not required for v1 and
  should not block launch.
- **Hard requirement carried forward**: no server-side key escrow, no
  "recovery contacts holding server-visible shares," and no scheme where
  ANKAI infrastructure alone (without the user's recovery key or another
  trusted device) can reconstitute a user's private keys.

### Abuse reporting without breaking E2EE for everyone else

Directly answers `docs/threat-model.md`'s "Repudiation vs. abuse reporting
tension" note: voluntary, consensual, per-message, and impossible for the
server to trigger non-consensually or retroactively.

- **Mechanism**: reporting is entirely recipient-initiated and scoped to a
  single message. When a user taps "Report," the client packages (a) the
  plaintext of that specific message — already decrypted locally, nothing
  new is revealed by reporting it; (b) minimal cryptographic context
  proving who sent it and in which conversation/epoch (the sender's MLS
  leaf signature and the message's AEAD ciphertext/tag); and (c) any
  additional surrounding messages the reporter explicitly chooses to
  include for context — never automatic, never more than what the user
  selects. This bundle goes directly to ANKAI's moderation backend over an
  authenticated channel. No other message and no other party's traffic is
  touched; the rest of the conversation stays fully E2EE and unreadable to
  ANKAI.
- **Non-forgeability requirement**: standard AEAD (plain AES-GCM or
  ChaCha20-Poly1305) is **not key-committing** — the "invisible
  salamanders" class of attack lets a malicious party craft a ciphertext
  that decrypts to different plaintexts under different keys, which would
  let a bad-faith reporter attempt to frame an innocent sender. Two
  non-custom paths to close this, either of which is an established,
  published construction rather than a new primitive:
  (a) a committing-AEAD wrapper from the message-franking literature
  (Bellare & Hoang / Grubbs et al.'s CTX/committing-AEAD constructions)
  layered on top of the AEAD OpenMLS already uses for application
  messages, or (b) rely on MLS's own transcript-hash chain and
  `exporter_secret`-derived binding, which already cryptographically ties
  each message to an exact group epoch and sender leaf — this may be
  sufficient on its own, but confirming that (vs. needing an explicit
  wrapper) is a dedicated pre-launch audit question, not assumed here.
- The moderation backend independently re-verifies the reported bundle's
  signature/commitment before acting on it — a report is only actionable
  if it cryptographically proves the claimed sender actually produced that
  exact ciphertext in that exact conversation, which prevents fabricated
  reports without granting the server visibility into anything else.

## Licensing summary (for the license manifest)

| Component | License | Notes |
|---|---|---|
| OpenMLS | MIT | Core protocol engine, 1:1 + group |
| `openmls_rust_crypto` | MIT/Apache-2.0 | Crypto backend for OpenMLS |
| `chacha20poly1305`, other RustCrypto AEAD crates | MIT/Apache-2.0 | File transfer, SFrame framing |
| `argon2` (RustCrypto) | MIT/Apache-2.0 | Recovery-key KDF |
| BLAKE3 | MIT/Apache-2.0/CC0 | Safety-number fingerprints |
| SQLCipher (Community Edition), via `rusqlite` | BSD-style, attribution required | Local DB encryption; commercial-use-safe |
| SQLite3 Multiple Ciphers (fallback) | MIT | Alternative if SQLCipher's OpenSSL vendoring is a concern |
| `keyring` crate | MIT/Apache-2.0 | OS secure-storage key management |

**Rejected on licensing grounds:**
- **`libsignal`** (AGPL-3.0-only, Signal Messenger LLC) — network copyleft;
  no known third-party commercial dual-license as of this research. Using
  it as a dependency would obligate releasing complete corresponding
  source of any combined program to every user reached over a network,
  conflicting with a closed marketplace backend. If ANKAI ever wants Signal
  Protocol specifically, the paths are a clean-room permissive
  reimplementation (extra audit cost) or a direct commercial negotiation
  with the Signal Foundation — neither pursued here.
- **RingRTC** (AGPL-3.0-only, Signal) — same reasoning; reference
  architecture only, not a dependency.

**Rejected on maturity/disclosure grounds (license was fine):**
- **vodozemac** (Apache-2.0) — see the dedicated section above; an
  unresolved, publicly disputed high-severity disclosure (Feb 2026) rules
  it out for now, not its license.

## Resolved: workspace license field

This ADR's research independently caught that `Cargo.toml`'s workspace had
briefly declared `license = "AGPL-3.0-only"` — a scaffolding default set
without justification, in tension with this ADR's copyleft-avoidance
guidance. That field has since been removed (repo is unlicensed / all-
rights-reserved in the interim). The actual open-source-vs-closed business
decision (spec section 42: what's open — protocol, crypto, client core,
SDKs — vs. closed/hosted — marketplace backend, abuse infra) is deferred to
its own dedicated ADR, tracked in `PROGRESS.md`. Every library recommended
in *this* ADR is permissively licensed (MIT/Apache-2.0/BSD-style), so
nothing here forces that future decision either way.

## Consequences

**Positive:**
- One audited protocol family (MLS / RFC 9420 via OpenMLS) covers 1:1,
  group, and multi-device instead of maintaining two separate crypto
  stacks — smaller audit surface, less code, real-world precedent from
  Wire and the Apple/Google RCS rollout.
- Fully permissive licensing (MIT/Apache-2.0/BSD-style) across the entire
  recommended stack — no AGPL/GPL entanglement forced by a crypto
  dependency, compatible with a closed marketplace backend regardless of
  how the open question above resolves.
- Multi-device and group membership share the same primitive, so "add a
  device" and "add a community member" are the same well-tested code path.
- The thin cloud never needs plaintext, private keys, or even conversation
  content — only opaque ciphertext ordering/relay — consistent with
  `docs/threat-model.md`'s hostile-curious-server assumption throughout.

**Negative / risks:**
- MLS requires a Delivery Service to totally order `Commit`s per
  conversation, including 1:1 DMs — an availability dependency on ANKAI's
  relay layer for conversation *state changes* that a pure asynchronous
  double ratchet wouldn't have. Needs explicit handling in ADR-0003 so it
  doesn't become a hard single point of failure.
- MLS-for-1:1 is a comparatively newer real-world pattern than the
  decade-plus battle-tested Signal Protocol double ratchet; less deployed
  history at ANKAI's specific traffic shape.
- SFrame-over-WebRTC (voice/video E2EE) has no mature off-the-shelf Rust
  crate; ANKAI must build and separately audit this integration.
- The abuse-report commitment scheme (key-committing AEAD / franking-style
  binding) is a composition ANKAI is responsible for building correctly —
  "invisible salamanders"-class bugs are subtle and this needs explicit
  audit attention, not an assumption that "AEAD is AEAD."
- OpenMLS has 2 small documented gaps versus full RFC 9420 compliance and,
  as of this research, 1 Low-severity finding from the March 2026 SRLabs
  audit still being addressed — must confirm resolution before relying on
  it for launch.
- The recovery model has an inherent, permanent "lose the recovery key,
  lose the backup" tradeoff by design. That's the point of not letting the
  server help recover — but it's a real support/UX cost the product team
  must own explicitly.

**Required before shipping to real users (gate, not optional):**
1. Independent professional security audit of ANKAI's specific
   integration work: MLS wiring, SQLCipher key management, SFrame/call-key
   derivation, file-transfer key wrapping, recovery-key crypto, and
   especially the abuse-report commitment scheme. Every individual
   protocol here is proven; the way ANKAI composes them together is not,
   and has not been reviewed by anyone outside this ADR.
2. Resolve OpenMLS's outstanding Low-severity audit finding and identify
   the "two small exceptions" to RFC 9420 compliance before depending on
   it for launch.
3. Wire a dependency-license scan (`cargo audit` / equivalent) into CI
   (per PROGRESS.md's later CI step) to catch any transitively-pulled
   copyleft dependency before it ships.
4. Revisit vodozemac's status periodically — if the disputed disclosure is
   resolved and re-audited, it remains a plausible permissively-licensed
   alternative for narrower use cases later.
