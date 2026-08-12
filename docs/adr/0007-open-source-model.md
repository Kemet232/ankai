# 7. Open-source model

Status: Accepted
Date: 2026-08-13

## Context

The product spec (section 42) asks which parts of ANKAI should be open vs.
closed, suggesting as a starting point:

- Open: protocol, crypto implementation, client core, theme SDK, provider SDK
- Closed/hosted: marketplace backend, abuse infrastructure, certain
  recommendation services

`docs/adr/0004-e2ee-stack.md` surfaced this as a real, undecided tension: a
scaffolding default had briefly set the workspace to AGPL-3.0, which that
ADR flagged as needing an explicit decision rather than an accident. This
ADR makes that decision.

Two considerations pull in the same direction here, which makes the call
easier than it might first appear:

1. **Trust.** ANKAI's entire private-messaging pitch rests on "the server
   operator cannot read your messages" (ADR-0004's hostile-curious-server
   assumption). That claim is only as credible as the ability for anyone —
   security researchers, journalists, a suspicious user — to actually read
   the code doing the encrypting. Signal, WhatsApp's crypto layer, and
   Matrix all publish their crypto/client code for exactly this reason, even
   while keeping server infrastructure closed.
2. **Ecosystem.** Section 41 of the product spec wants ANKAI to eventually
   support third-party clients via a stable, versioned protocol. A protocol
   nobody can implement against without a closed reference client is not
   really open — the protocol and the reference implementation of it need
   to be available together. The creator marketplace flywheel (product spec
   section 58) similarly depends on the Theme SDK and Provider SDK being
   freely buildable-against, or third-party creators/plugin authors can't
   participate.

## Decision

**Open, under Apache-2.0:**

- `core` (`ankai-core`) — identity, crypto integration, protocol types,
  local persistence, networking. This is where the E2EE integration work
  ADR-0004 requires an independent security audit for lives; it must be
  publicly readable for that audit's results to mean anything to a user who
  wasn't there for it.
- `client` — the native desktop application itself.
- `docs/protocol/` — the wire-format/schema specs, once written (Phase 1+).
- Theme SDK and Provider SDK — both the formal spec and a reference
  implementation. A closed SDK would directly undermine the creator
  marketplace loop these exist to enable.

**Closed/hosted, proprietary:**

- The marketplace backend (listings, search, payments, entitlements,
  creator payouts, package signing authority).
- Abuse/moderation infrastructure and its detection heuristics — publishing
  spam/abuse detection logic mostly helps whoever is trying to evade it.
- The recommendation/trending-engine's internal scoring implementation
  (the product spec's section 18 wants trending to be *explainable* to
  users — "why is this trending" — which is a UI/transparency requirement,
  not the same thing as the ranking code being open source; it can stay
  closed while still showing its work).
- The specific central-services implementation (account/auth/discovery
  servers) — the *protocol* they speak is open per above, but the server
  binary itself doesn't need to be, partly for the ordinary reason (it's
  not useful to anyone without ANKAI's user graph) and partly to raise the
  bar on studying it for attack surface.

## Why Apache-2.0 specifically, and not AGPL/GPL or MIT

- **Not AGPL/GPL for `core`/`client`**: this is the tension ADR-0004
  flagged. Even setting aside the (genuinely disputed) question of whether
  a desktop app that merely talks to a separate marketplace backend over a
  network would actually trigger AGPL's network-copyleft clause on that
  backend, betting the company's licensing posture on winning an untested
  "mere aggregation vs. combined work" argument is an unforced risk. Every
  dependency ADR-0004 recommends is already permissively licensed
  specifically to avoid this fight — this ADR keeps ANKAI's own code
  consistent with that choice rather than reintroducing the exact risk the
  dependency selection worked to avoid.
- **Not plain MIT**: Apache-2.0 gives an explicit patent grant (and patent-
  retaliation clause) that MIT doesn't. That matters more for ANKAI than
  most projects, because ANKAI will likely hold real patentable-or-not trade
  secrets around the trending engine, netplay matchmaking, and similar —
  Apache-2.0's patent grant protects downstream users of the *open* parts
  without ANKAI needing to make the same commitment about the closed parts.
  It's also the de facto standard for serious corporate-backed open source
  (Rust itself is dual MIT/Apache-2.0), which matters for a project that
  wants outside contributors and eventual enterprise/partner trust.
- **Not a copyleft-with-CLA hybrid**: adds legal/process overhead (every
  contributor needs to sign something) that isn't justified at pre-alpha
  scale with a small team. Revisit if/when outside contribution volume
  actually warrants it.

## What this does NOT decide yet

- Exact timing of accepting outside contributions (the repo is public, but
  "public and readable" and "open for PRs" are different states — see
  README's current "Contributing" section).
- Whether specific non-crypto subsystems added later (e.g. a future
  recommendation-explanation UI, or a reference "ANKAI Node" relay
  implementation) should also be open — default to Apache-2.0 unless a
  future ADR argues otherwise for a specific case, consistent with this
  ADR's reasoning.
- Whether ANKAI ever pursues a dual-license/commercial-support model like
  Slint's (ADR-0002) — not needed now, worth considering only if a
  legitimate reason emerges (e.g. an enterprise/white-label offering).

## Consequences

**Positive:**
- Resolves ADR-0004's flagged tension with a real answer instead of an
  accidental default.
- The E2EE audit ADR-0004 requires before shipping to real users is
  actually meaningful to outside observers, not just internally reassuring.
- Unblocks third-party client and provider/theme ecosystem work later
  (product spec sections 41, 21, 31) without a future relicensing fight.
- Simple, well-understood, single license across all open components —
  easy to explain in a license manifest, easy for contributors and
  investors/acquirers to evaluate.

**Negative / risks:**
- Apache-2.0 permits commercial forks/competitors to use ANKAI's client and
  core code freely, including the crypto integration work. Accepted:
  the trust and ecosystem benefits outweigh this, and the actual hard-to-
  replicate value (community, marketplace liquidity, central
  infrastructure, brand) isn't in the client code anyway.
- Publishing `core` means publishing the E2EE integration code before its
  required security audit (ADR-0004) is complete. This is intentional, not
  an oversight — open review is part of how that audit gets taken
  seriously — but it means any pre-audit vulnerabilities are visible to
  attackers too. Mitigation: don't ship real user data/production
  credentials through pre-audit code; treat "public repo" and "production
  ready" as separate gates, which PROGRESS.md's phase tracking already
  does.
- A LICENSE file now formally invites external use of pre-alpha,
  architecture-only code. Low risk at this stage (there's no working
  product yet to fork), but worth remembering as the codebase matures.

## Follow-up actions taken alongside this ADR

- Added `LICENSE` (Apache-2.0) at the repo root.
- Set `license = "Apache-2.0"` in the workspace `Cargo.toml`'s
  `[workspace.package]` (previously intentionally unset — see
  `docs/adr/0004`'s resolved note and `PROGRESS.md`'s prior open item).
