# ANKAI — Progress / Resume Point

> **If you are picking this up cold (new session, ran out of context, etc.):**
> Read this file top to bottom, then skim `docs/adr/*.md` for decisions already locked in.
> That's enough to resume without re-reading the full product spec.

Last updated: 2026-08-13

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

**Phase 0 — Architecture.** Nothing user-facing built yet. In progress:

| Track | Status | Owner artifact |
|---|---|---|
| Repo scaffold | done | this repo |
| ADR process | done | `docs/adr/0001-*.md` |
| Native UI stack decision | researching | `docs/adr/0002-native-ui-stack.md` |
| P2P networking stack decision | researching | `docs/adr/0003-p2p-networking-stack.md` |
| E2EE stack decision | researching | `docs/adr/0004-e2ee-stack.md` |
| Emulation/licensing approach | researching | `docs/adr/0005-emulation-integration.md` |
| Threat model | not started | `docs/threat-model.md` |
| Design tokens (Liquid Y2K) | not started | `docs/design/tokens.md` |
| Rust workspace skeleton | not started | `core/`, `client/` |
| CI | not started | `.github/workflows/` |

## Immediate next steps (in order)

1. Get `gh` authenticated (user action — see below), create GitHub repo, push.
2. Read back the 4 ADR drafts once research agents finish; accept or revise them as
   ADR status `Accepted`, commit.
3. Write `docs/threat-model.md` (STRIDE-style pass over identity, messaging, P2P,
   marketplace).
4. Start Rust workspace: `core` crate (identity, crypto, db, protocol types) +
   `client` crate (native shell using whatever ADR-0002 picks).
5. Stand up minimal CI (fmt, clippy, test) in `.github/workflows/ci.yml`.
6. Begin Phase 1 (native shell: window, nav, theme foundation, SQLite, settings)
   per the phase list — see any ADR or ask the user for the full phase breakdown
   if it's not already summarized in this file by then.

## Decisions locked so far

_(none yet — first ADRs still in flight; update this list as ADRs move to Accepted)_

## Open questions for the human

- Repo is private under the `Kemet232` GitHub account — say if you want it
  moved to an org or made public later.

## How to resume seamlessly (read this if you're a future/fresh session)

1. `cd ~/Projects/ankai && git log --oneline -20` — see what actually landed.
2. Read this file (`PROGRESS.md`) — it's kept current after every work session.
3. Read `docs/adr/` — every non-trivial technical choice is recorded there with
   rationale, not just the conclusion. Don't re-litigate an Accepted ADR without
   new information.
4. Check the "Immediate next steps" list above and continue from the first
   undone item.
5. If the user just says "continue ANKAI" with no other context, that's enough —
   do not re-ask them to re-explain the product.
