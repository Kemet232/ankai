# 1. Record architecture decisions

Status: Accepted
Date: 2026-08-13

## Context

ANKAI has many open technical questions with real tradeoffs (UI toolkit, P2P
stack, crypto libraries, emulation approach, etc.). Decisions and their
rationale need to survive across sessions, contributors, and (for this
project specifically) across context-window resets of the AI agents doing
the building.

## Decision

We use lightweight Architecture Decision Records, one per significant
decision, numbered sequentially in `docs/adr/`. Each ADR has:

- **Status**: Proposed / Accepted / Superseded by ADR-000N
- **Context**: the problem and constraints
- **Decision**: what we chose
- **Consequences**: what this makes easier/harder, what we're trading away

An ADR is only binding once its status is `Accepted`. Don't re-litigate an
Accepted ADR without new information — supersede it explicitly instead.

## Consequences

Anyone (human or agent) resuming this project reads `PROGRESS.md` then
`docs/adr/*.md` to get full context on "why is it built this way" without
needing the original conversation history.
