# ANKAI Threat Model (v0)

Status: Draft — living document, revisit every time a subsystem in scope here
ships. This is a first pass covering the systems whose architecture is already
decided or in flight; it will grow ADR by ADR, not all at once.

## Approach

STRIDE per subsystem (Spoofing, Tampering, Repudiation, Information disclosure,
Denial of service, Elevation of privilege), scoped to what's architecturally
committed so far. Security-sensitive subsystems (crypto, P2P, marketplace
packages, theme sandbox) require review by an agent/person other than the one
that implemented them — see `docs/adr/0001` process note; this applies equally
to security review.

## Assets to protect

- Private message plaintext (1:1 and group)
- Account private keys / device keys
- User IP addresses (from untrusted peers, absent explicit consent)
- Marketplace payment/entitlement data
- Community moderation state (ban lists, reports) — integrity, not secrecy
- Creator payout information
- Local device: message DB, credentials, session tokens

## Trust boundaries

1. **Client ↔ ANKAI central services** (accounts, discovery, moderation,
   marketplace metadata, signalling).
2. **Client ↔ Client, direct P2P** (untrusted by default — any peer, friend
   or stranger, is an untrusted process from a security standpoint; social
   trust ("friend") is not the same as security trust).
3. **Client ↔ ANKAI Node** (community-run relay — semi-trusted; operator is a
   pseudonymous third party, not ANKAI infrastructure).
4. **Client ↔ third-party provider plugin** (untrusted code, sandboxed).
5. **Client ↔ marketplace theme package** (untrusted content, sandboxed +
   signature-verified).

## Threats by subsystem

### Identity & authentication

- **Spoofing**: account takeover via credential stuffing → mitigate with
  passkeys as primary auth (see future ADR), rate limiting, device
  verification on new-device login.
- **Elevation**: forged device registration → device keys must be signed by
  the account's root identity key; server never issues device trust on its
  own authority.
- **Repudiation**: user disputes a marketplace purchase or moderation action →
  central services need auditable (not necessarily public) logs for
  purchases/moderation actions specifically; this is explicitly exempt from
  "don't log everything" — it's operational necessity, not surveillance.

### Private messaging (E2EE)

- **Information disclosure**: server operator is explicitly assumed hostile-
  curious for message content — plaintext must never reach central
  infrastructure. Covered in depth once `docs/adr/0004-e2ee-stack.md` lands;
  this threat model will be updated to reference it directly.
- **Tampering**: message ordering/replay attacks by a malicious relay →
  ratchet protocols provide this; relays must be architecturally incapable of
  modifying ciphertext undetected (AEAD).
- **Repudiation vs. abuse reporting tension**: users need to voluntarily
  disclose specific messages for moderation without breaking E2EE for
  everyone — this is a deliberate, user-initiated exception, not a backdoor.
  Must be designed so it cannot be done non-consensually or retroactively by
  the server.

### P2P networking

- **Information disclosure (IP address)**: default P2P connections leak IP to
  the peer. Untrusted/stranger connections must default toward relay unless
  the user has chosen "Automatic" or "Direct Only" privacy mode with informed
  understanding of the tradeoff. Friends-only direct connections are lower
  risk but still worth disclosing in UI ("this will connect you directly").
- **Denial of service**: malicious peer floods a client with connection
  attempts or garbage traffic → rate limiting, connection attempt caps,
  ability to fully block a peer identity at the protocol level (not just UI).
- **Spoofing peer identity**: P2P connections must be authenticated to a
  cryptographic identity (public key), never trust IP+port alone, to prevent
  MITM/impersonation on hostile networks.
- **Elevation via relay**: a malicious "ANKAI Node" relay operator could
  attempt traffic analysis, selective dropping, or (if it can) MITM. Relays
  must only ever see ciphertext for private content; relay operators are
  explicitly untrusted for confidentiality, only trusted for availability.

### Marketplace / theme packages

- **Tampering**: a compromised CDN/mirror/community-cache serves a modified
  theme package → packages must be content-addressed and signature-verified
  (creator signing key) before install; hash mismatch = hard reject, no
  "install anyway."
- **Elevation of privilege**: a theme or provider plugin attempts to escape
  its sandbox to read local files, network, or other app data → themes are
  declarative-only (no arbitrary code execution) per product requirement;
  provider plugins are treated as fully untrusted code requiring an explicit
  capability-grant model (e.g., "this provider wants: internet access to
  domain X"), default-deny.
- **Fraud**: fake creator accounts, stolen artwork resold, chargeback abuse →
  out of scope for this technical threat model; covered by marketplace
  moderation/policy (product spec section on copyright & moderation).

### Local data

- **Information disclosure**: device theft/loss exposes local message DB →
  local message history should be encrypted at rest, keyed to
  OS-level secure storage (Keychain/Credential Manager/Secret Service) or a
  user passphrase, not stored in plaintext SQLite.
- **Tampering**: another local process/user on a shared machine modifying
  local state → standard OS file permissions; not defending against a
  fully-compromised local OS (out of scope — that's a lost endpoint).

## Explicitly out of scope (for now)

- Nation-state-level traffic analysis resistance (no Tor-equivalent guarantee
  — ANKAI is not positioning itself as an anonymity network).
- Physical security of user devices.
- Supply-chain compromise of the Rust toolchain / crates.io itself (mitigated
  generically via `cargo audit` / dependency pinning in CI, not modeled
  further here).

## Open items requiring dedicated review later

- Full review once `docs/adr/0004-e2ee-stack.md` (crypto) lands.
- Full review once `docs/adr/0003-p2p-networking-stack.md` (P2P/relay) lands.
- Independent security review required before any crypto code ships to real
  users — this document is not a substitute for that.
