# ANKAI

A native (no webview) social platform for anime and gaming communities —
profiles, communities, E2EE messaging, watch-party "Hangouts", P2P-first
networking, an emulation frontend, and a creator marketplace.

Design language: **Liquid Y2K** — Y2K/Aqua-era tactility meets modern
translucent glass materials, personal MySpace-style profile customization,
native smooth animation. Not a SaaS dashboard, not Discord with a skin.

Architecture: **thin cloud, fat client**. Central infrastructure handles
identity, discovery, moderation, and marketplace metadata; expensive traffic
(messages, voice, video, watch parties, theme assets) prefers direct
peer-to-peer connections, falling back to relays only when needed.

## Status

Pre-alpha. Phase 0 (architecture). See [`PROGRESS.md`](./PROGRESS.md) for the
current state and next steps — **read that file first** if you're resuming
work on this project.

## Repo layout

```
core/     Rust workspace: identity, crypto, local db, protocol types,
          networking — shared by all future clients (desktop first, mobile later)
client/   Native application shell (UI toolkit decided in docs/adr/0002)
docs/
  adr/        Architecture decision records
  protocol/   Wire formats, schemas
  design/     Design tokens, component specs
```

## Non-negotiables

- No Electron, no webview, no embedded Chromium as the app shell.
- No invented cryptography — established protocols/libraries only.
- No blockchain, no tokens, no cryptocurrency.
- ANKAI does not distribute ROMs or copyrighted BIOS files.
- Creators keep 90% of marketplace sales.

## License

Apache-2.0 (see `LICENSE`) for `core`, `client`, the protocol, and the
Theme/Provider SDKs — chosen so ANKAI's E2EE integration and client code
stay independently auditable, and so third-party clients/plugins can build
against the SDKs freely. ANKAI's central marketplace backend, abuse
infrastructure, and recommendation-engine internals are closed. Rationale
in [`docs/adr/0007-open-source-model.md`](./docs/adr/0007-open-source-model.md).

## Contributing

Not yet open for external contribution — pre-alpha, architecture in flux.

Release packaging for the no-install libmpv runtime is documented in
[`docs/releasing-libmpv.md`](./docs/releasing-libmpv.md). The release scripts
reject default/GPL mpv builds; release CI must run the bundled dependency-
closure verifier on each native target.
