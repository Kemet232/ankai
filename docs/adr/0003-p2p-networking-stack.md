# 3. P2P networking stack

Status: Accepted

Date: 2026-08-13

## Context

ANKAI is "thin cloud, fat client": central ANKAI servers should be limited to
identity/auth, discovery, signalling, and moderation metadata. All actual
traffic should prefer to travel peer-to-peer:

- 1:1 and group E2EE DMs
- Voice, video, screen share
- File transfer
- Watch-party ("Hangout") playback-state sync
- Presence, typing indicators, reactions
- Theme/profile asset distribution
- Emulator netplay traffic
- Game spectating

Connection preference order, strongest to weakest:

1. Direct P2P (no intermediary touches payload or metadata)
2. NAT traversal / hole-punching (still direct once established)
3. Trusted/community relay — "ANKAI Node," a voluntary, user-run
   relay/cache box with operator-set bandwidth/storage/CPU caps, no
   crypto/blockchain incentive layer
4. Central ANKAI relay — last resort, ANKAI-operated infrastructure

ANKAI also needs a user-facing **network privacy mode**:

- **Automatic** — prefer direct, fall back to relay (community, then
  central) when needed.
- **Maximum Privacy** — prefer relay for untrusted/non-contact peers even
  when direct would work.
- **Direct Only** — never use ANKAI's own central relay infrastructure
  (direct P2P, hole-punching, and community ANKAI Nodes only).

Hard requirement: **public strangers must not automatically learn each
other's raw IP address** without the user opting into that tradeoff (e.g. by
adding someone as a contact/friend, or explicitly accepting a direct
connection prompt). This means "attempt direct connection" cannot be the
unconditional default for arbitrary peers — it has to be gated by
per-peer trust level, which is an application-layer policy on top of
whatever P2P library we pick, not something any library gives us for free.

Beyond 1:1 traffic, ANKAI needs group voice/video that works from a 2-person
DM call up to larger community voice rooms, and needs to support emulator
netplay (rollback-style, latency-sensitive, small unreliable state packets)
and game spectating (mostly one-to-many state fan-out).

This ADR was researched by web search in August 2026 against the current
(2026) state of: rust-libp2p, QUIC via `quinn`, WebRTC via `webrtc-rs` /
`str0m` vs. C++ `libwebrtc`, `iroh`, STUN/TURN options (`coturn`), and
hole-punching techniques, plus prior art on adaptive group-call topology
(mesh → SFU) from LiveKit, mediasoup, and Tailscale's NAT traversal work.

## Decision

ANKAI uses **two complementary transport stacks**, not one unified stack,
because the traffic types have genuinely different needs: structured
data/control traffic wants a general-purpose reliable+unreliable P2P
transport with content addressing, while voice/video/screen share wants a
mature, purpose-built real-time media stack with echo cancellation, noise
suppression, and jitter buffering already solved.

### Traffic-type → transport mapping

| Traffic | Transport | Why |
|---|---|---|
| 1:1 / group E2EE DMs | iroh (QUIC) reliable streams | ordered, encrypted-at-transport, dial-by-key |
| File transfer | iroh-blobs (BLAKE3 content-addressed) over QUIC | resumable, verifiable, cacheable by relays without trusting them |
| Theme/profile asset distribution | iroh-blobs, same content-addressing | community ANKAI Nodes can cache/serve without being trusted — hash mismatch = reject |
| Hangout playback-state sync | iroh-gossip (pub/sub) or QUIC datagrams for high-frequency ticks | small, frequent, loss-tolerant state |
| Presence / typing / reactions | iroh-gossip | pub/sub fan-out to a room/community without a server relay |
| Emulator netplay | QUIC unreliable datagram extension, direct on the iroh QUIC connection | GGPO-style rollback netcode wants raw, low-latency, loss-tolerant datagrams — not a full WebRTC stack |
| Game spectating | QUIC streams/datagrams, gossip fan-out for larger audiences | mostly state broadcast, same primitives as playback sync |
| Voice | WebRTC (webrtc-rs / str0m) | mature AEC/NS/AGC, jitter buffer, Opus |
| Video / screen share | WebRTC | mature simulcast/SVC, congestion control (GCC), codec negotiation |
| Signalling for WebRTC session setup | Central ANKAI server (existing "thin cloud" channel) or iroh stream as the signalling transport | small messages, already-trusted channel |

### Core P2P layer: iroh (QUIC-native), not raw `quinn`, not `rust-libp2p`

We adopt **iroh** (`n0-computer/iroh`, MIT/Apache-2.0 dual-licensed, iroh 1.0
shipped June 2026) as the generic P2P layer for everything that isn't
real-time audio/video:

- Iroh dials peers **by public key**, not IP address, and handles NAT
  traversal, relay-assisted rendezvous, and fallback-to-relay internally.
  This maps directly onto our "identity/discovery centralized, everything
  else P2P" model: ANKAI's central servers hand out peer public keys and
  connection hints (discovery/signalling), iroh does the rest.
- Hole punching happens **inside the QUIC handshake** via iroh's own QUIC
  implementation ("noq"), which implements a NAT-traversal draft extension.
  Per n0's own description this is the first production-grade
  implementation of QUIC-native NAT traversal, as opposed to the vanilla
  `quinn` crate, which has no built-in NAT traversal — you'd have to bolt
  that on yourself (which is effectively what iroh did).
- `iroh-relay` is the relay implementation n0 runs in production, and it is
  **open source and self-hostable**. This is a direct, ready-made fit for
  the "ANKAI Node" concept (see below) and for ANKAI's own central relay
  tier — both tiers speak the same protocol, just at different trust/SLA
  levels.
- `iroh-blobs` gives content-addressed (BLAKE3), resumable, arbitrarily
  large blob transfer. This is exactly what file transfer and theme/asset
  distribution need, and it's what makes community-node caching *safe*:
  a node can serve or cache a blob without ANKAI or the recipient trusting
  the node's integrity, since the hash is verified client-side.
- `iroh-gossip` gives topic-based pub/sub sized for "an average phone,"
  which fits presence/typing/reactions/playback-sync fan-out inside a DM,
  group, or Hangout room without needing a server in the loop.
- Dual MIT/Apache-2.0 license, no licensing friction.

**Why not `rust-libp2p` as the primary layer:** libp2p is mature and
well-proven (IPFS, Filecoin/Forest, Ethereum consensus clients like
Lighthouse all run on it in production), dual MIT/Apache-2.0 licensed, and
has real building blocks we considered — QUIC + TCP transports, relay-v2,
DCUtR for hole-punching, gossipsub for pub/sub, and recent WebTransport
support for browser interop. But it's built around a swarm/DHT-oriented
model (peer discovery via Kademlia DHT, multi-transport negotiation) that
solves a problem ANKAI doesn't have — our discovery is centralized by
design. Its hole-punching (DCUtR) and gossipsub-over-relay paths also have
open rough edges documented in libp2p's own GitHub discussions as of 2026
(e.g. gossipsub message delivery issues over relayed connections, DCUtR
failures at the upgrade stage in certain NAT configurations). iroh's
narrower, opinionated "dial by key, punch through, fall back to relay"
model is a tighter fit for what we actually need and is simpler to reason
about for the privacy-mode gating logic described below.

We are not fully closing the door on libp2p: a `libp2p-iroh` bridging crate
exists, and if `iroh-gossip` or iroh's relay network doesn't scale the way
we need, gossipsub is a credible fallback for the pub/sub layer
specifically. The P2P layer should sit behind an internal ANKAI trait/module
boundary in `core/` so this is a swappable implementation detail, not a
load-bearing assumption baked through the codebase.

**Why not raw `quinn` directly:** `quinn` is the widely-used, mature,
general-purpose Rust QUIC implementation (86M+ downloads) and is in fact
what iroh's own QUIC layer is descended from. But vanilla QUIC has no NAT
traversal story — that's an application concern we'd have to build
ourselves, which is precisely the hard, security-sensitive work iroh has
already done and hardened in production. Building it ourselves would be
reinventing iroh, badly.

### Real-time media: WebRTC (webrtc-rs / str0m), not homegrown, not libp2p

For voice, video, and screen share we use **WebRTC**, not QUIC data
channels or libp2p streams, because WebRTC's value isn't the transport —
it's the mature real-time media pipeline: echo cancellation, noise
suppression, automatic gain control, jitter buffering, simulcast/SVC, and
congestion control (GCC). Reimplementing that on top of a generic transport
would be a multi-year effort on its own and is explicitly out of scope.

Within Rust's WebRTC options:

- `webrtc-rs` shipped a **Sans I/O, runtime-agnostic redesign at v0.20**,
  now the recommended line for new projects; the older Tokio-coupled
  v0.17.x line is bug-fix-only. This is a meaningful maturity jump from
  its earlier callback-heavy, lock-heavy design that made it awkward to
  use idiomatically in Rust.
- `str0m` is a from-scratch Sans I/O WebRTC implementation built
  specifically because the maintainers found webrtc-rs's earlier design
  a poor fit for Rust. It's heavily used and tested as the media engine
  inside an SFU; peer-to-peer usage is supported but less battle-tested.
  (libp2p itself considered switching from webrtc-rs to str0m for its own
  WebRTC transport, which is a useful maturity signal either way.)
- C++ `libwebrtc` (what Chrome/Edge/Safari's WebRTC and most commercial
  SFUs are built on) remains the most battle-tested implementation and is
  reportedly in "house-cleaning mode" in 2026 — few new features, steadily
  fewer bugs. It is the safe fallback if the Rust implementations prove
  insufficient for audio quality, but pulling in a C++ dependency cuts
  against ANKAI's Rust-native, memory-safety-first posture, so it is not
  the starting choice.

**Decision:** start with `webrtc-rs` v0.20 for the client-side P2P voice/
video/screen-share path, and evaluate `str0m` specifically for the SFU-side
component described below (its production track record is stronger there).
Both are Sans I/O, so this isn't a one-way door — swapping the media engine
without rewriting the surrounding async plumbing is realistic if webrtc-rs's
audio quality doesn't hold up in the early prototype (see Spikes below).

ICE/STUN/TURN for WebRTC uses standard STUN for reflexive-address discovery
and **`coturn`** for TURN relay when ICE can't find a direct/hole-punched
path. Coturn is free, open-source, and the de facto production standard for
self-hosted STUN/TURN; it's what both ANKAI's central relay tier and,
optionally, ANKAI Nodes run for the media-relay case (separate from
`iroh-relay`, which handles the generic-data-P2P relay case).

### NAT traversal

Two independent hole-punching paths, one per stack:

- **Generic P2P (iroh):** hole punching happens inside the QUIC handshake,
  rendezvous-assisted by an `iroh-relay` (community ANKAI Node or central
  ANKAI relay). If punching succeeds, the relay steps out of the data path
  entirely; if it fails, the same relay continues forwarding, so there's no
  separate "give up and reconnect" step — connectivity degrades gracefully
  from the peer's point of view.
- **WebRTC media:** standard ICE (STUN for reflexive candidates, TURN via
  coturn as relay candidate) does the equivalent job for voice/video/
  screen-share sessions specifically.

Published success rates vary by methodology: Tailscale reports direct
(hole-punched) connectivity **north of 90%** in typical conditions using
heavily optimized techniques; a broader 2026 academic measurement study
across 4.4M+ attempts and 85K+ networks found a more conservative **~70%
(±7%)** baseline success rate for generic hole-punching, with symmetric
("hard") NATs as the dominant failure mode — these randomize the outbound
port mapping per connection, which defeats prediction-based punching
techniques outright. ANKAI should plan capacity assuming something closer
to the conservative number until we have our own measurements (see Spikes),
not the vendor-optimized upper bound.

### Relay tiers and the "ANKAI Node"

Both the generic-data path and the media path have the same three-tier
relay structure, matching the four-tier connection preference order in the
Context section:

1. Direct / hole-punched — no relay involved.
2. **ANKAI Node** (community, voluntary) — a lightweight package a user can
   run that bundles:
   - an `iroh-relay` instance (rendezvous + fallback packet relay for the
     generic P2P/data stack: messaging, gossip, file transfer, netplay)
   - a `coturn` instance (TURN relay for WebRTC media)
   - optionally, a lightweight Rust SFU worker for group voice/video (see
     below)
   - a content cache for `iroh-blobs`-addressed data (theme/profile assets,
     shared files) — safe to cache because content addressing means a
     tampering or corrupt node just gets its cached copy rejected on hash
     mismatch, not trusted blindly
   
   Bandwidth, storage, and CPU are capped by the operator (no crypto/
   blockchain incentive layer in v1 — this is an altruistic/community-
   reputation model, consistent with the directive). Because both the
   community and central relay speak the exact same protocol
   (`iroh-relay` / coturn), a Node is a drop-in, lower-trust substitute for
   ANKAI's own relay, not a separate system to design and validate twice.
3. **Central ANKAI relay** — ANKAI-operated `iroh-relay` + coturn (+ SFU)
   cluster, last resort. Exists to guarantee connectivity when neither
   direct nor community paths are viable (symmetric-NAT peers, sparse
   Node coverage early on, or a user's privacy mode allows it).

### Group voice/video topology: mesh → elected relay → SFU

Naive full mesh doesn't scale (N-1 upload streams per participant), but
always using an SFU is unnecessary overhead/cost for a 2-3 person call.
ANKAI adopts an adaptive topology, consistent with how LiveKit and similar
platforms describe scaling WebRTC:

- **Small groups (≈2–5 participants):** full mesh, direct/hole-punched
  WebRTC connections between all participants. No relay needed unless a
  specific pair can't traverse NAT, in which case only that pair falls
  back to relay while the rest of the mesh stays direct.
- **Medium/larger rooms:** switch to a single forwarding point rather than
  full mesh. Two variants, both worth prototyping before locking in one:
  - **Elected-peer-relay:** one participant (highest bandwidth/CPU,
    stable connection) forwards media to the rest, avoiding a dedicated
    server for moderate room sizes. Requires a re-election/handoff
    protocol if that peer leaves or degrades — real complexity, not free.
  - **SFU fallback:** a lightweight Rust SFU (str0m-based, since str0m's
    strongest production use case today is exactly server-side SFU
    forwarding) runs either on an ANKAI Node or the central relay tier.
    The SFU only forwards encrypted media — it does not need to decrypt —
    provided ANKAI implements frame-level E2E encryption (an
    Insertable-Streams-equivalent) on top of the transport-level DTLS-SRTP
    WebRTC already provides, so the E2EE guarantee for voice/video holds
    even when a semi-trusted community Node is doing the forwarding. This
    is nontrivial and is explicitly called out as a spike item below.
- The threshold participant count for switching topology, and whether
  elected-peer-relay is worth its added complexity over "just use an SFU
  above N," are open questions to resolve empirically (see Spikes).

### Network privacy modes

The P2P libraries above will happily attempt a direct/hole-punched
connection to anyone by default — that default is wrong for ANKAI's public-
stranger case. Privacy mode is therefore enforced as an **application-layer
policy in the ANKAI connection manager** (in `core/`), sitting above iroh
and WebRTC, gating *whether an escalation to direct connection is even
attempted* per peer, based on trust level (contact/friend vs. stranger) and
the user's selected mode:

- **Automatic:** contacts/friends get direct P2P + hole-punching by
  default. Non-contacts start relay-only (community Node preferred, central
  as fallback) for the first interaction; escalate to direct only once the
  user has some trust signal (accepted a DM request, joined a shared
  community, etc. — exact rules belong in the threat model, not this ADR).
- **Maximum Privacy:** relay-preferred for any peer not on the user's
  trust list, even if direct would work — trades latency/relay load for
  never exposing IP to anyone outside an explicit relationship.
- **Direct Only:** direct P2P + hole-punching + community ANKAI Nodes only;
  the client refuses to use ANKAI's own central relay infrastructure at
  all. Users in this mode accept degraded connectivity in symmetric-NAT-vs-
  symmetric-NAT cases with no reachable community Node — there is no
  guaranteed-availability fallback by design.

Because both relay tiers speak the same protocol, none of the three modes
requires different wire protocols — only different policy about which tier
the connection manager is allowed to pick and when it's allowed to attempt
direct connection at all.

## Consequences

**De-risked by this decision:**

- QUIC-native NAT traversal (iroh) plus WebRTC ICE gives ANKAI two
  independently mature hole-punching paths rather than one homegrown one;
  most traffic should converge to genuinely P2P connections once the
  system is healthy, which is the core cost and privacy goal.
- Using WebRTC for voice/video/screen-share means we inherit years of
  audio DSP work (AEC/NS/AGC, jitter buffering, codec/bandwidth adaptation)
  instead of building it — this is the single highest-leverage "don't
  reinvent it" call in this ADR.
- Content-addressed blob distribution (`iroh-blobs`, BLAKE3) makes
  community-node caching of theme/profile assets and files safe by
  construction: a malicious or misconfigured Node can at worst withhold
  data, not corrupt it, since clients verify hashes.
- `coturn` for TURN is a known, boring, battle-tested component — no
  research risk there, mainly ops/cost planning (see below).
- The central-relay and community-Node tiers being protocol-identical
  (`iroh-relay`/coturn on both) avoids building and maintaining two
  separate relay implementations.

**Still risky / open — do not treat as settled:**

- This is **two networking stacks**, not one: two NAT-traversal
  implementations, two relay/TURN systems, two sets of failure modes, and
  privacy-mode gating logic that has to be enforced correctly across both.
  More integration surface than a single unified stack would have.
- iroh is comparatively new — 1.0 shipped June 2026. It doesn't yet have
  libp2p's multi-year, multi-large-project production pedigree (IPFS,
  Filecoin, Ethereum consensus clients). We're taking on ecosystem-maturity
  risk in exchange for a much better architectural fit. The internal
  P2P-layer abstraction called out above exists specifically to make a
  later swap to libp2p (or a fork of it) survivable if iroh churns or
  stalls.
- Symmetric-NAT peers are a hard floor on P2P-only connectivity —
  conservative measurements put generic hole-punch failure around 30%,
  not the ~10% vendor-optimized number. Central + community relay capacity
  must be sized for that floor, not assumed away.
- Frame-level E2E encryption compatible with SFU forwarding
  (Insertable-Streams-equivalent in Rust/webrtc-rs or str0m) is genuinely
  hard to get right; getting it wrong either silently breaks the E2EE
  guarantee for group calls or breaks the media pipeline (AEC/jitter
  buffering can be sensitive to what layer encryption sits at). This is
  the highest-risk single component in this ADR and needs its own spike
  and, later, its own security review — not just this ADR's sign-off.
- Elected-peer-relay adds a re-election/handoff protocol that full mesh or
  a fixed SFU don't need. It may not be worth the complexity; the
  threshold and even whether to build it at all should be decided from
  prototype data, not assumed.
- No crypto/blockchain incentive for ANKAI Node operators (by design) means
  community relay/cache capacity may be thin or geographically uneven
  early on. Central relay sizing for the first 6-12 months should assume
  the community layer contributes little, then be revisited as Node
  adoption grows — do not build a cost model that assumes early
  community offload.
- QUIC unreliable datagrams for emulator netplay are a less-trodden path
  than classic UDP rollback netcode (GGPO and derivatives); integrating
  rollback netcode against QUIC datagrams over an iroh connection needs
  real validation against an actual emulator core before it's load-bearing
  for the netplay feature.

**Infra cost implications:**

- Central relay cost is bounded by design (last resort, protocol-shared
  with community tier) but not zero: coturn alone is roughly $200+/month
  for a small multi-region cluster plus ~$0.09/GB egress at commodity
  cloud pricing, before ANKAI-specific scale. Video calls run ~1-3 Mbps/
  participant, so central-relay-mediated group calls are the expensive
  case to minimize via the mesh/elected-relay/SFU topology work above.
- iroh-relay operational cost is comparatively cheap for the generic-data
  path (mostly small messages, gossip, and blob-transfer overflow) but
  file-transfer and theme-asset-distribution volume through the central
  tier should be watched closely until community-Node caching has real
  adoption — that's the whole point of `iroh-blobs`' content addressing,
  and if Node adoption stays low, central relay absorbs that load instead.
- Running TURN/relay infrastructure has a real ongoing ops cost beyond
  hosting — TLS renewal, capacity planning, CVE patching — industry
  estimates run 15-20 engineer-hours/month for coturn alone at moderate
  scale. This is recurring operational burden, not a one-time build cost.

**Complexity tradeoffs accepted:**

- Two stacks instead of one, in exchange for not compromising either the
  generic-data P2P story or voice/video quality.
- Application-layer trust/privacy-mode gating instead of relying on
  library defaults, in exchange for actually meeting the "no automatic IP
  disclosure to strangers" requirement, which no off-the-shelf library
  guarantees out of the box.
- Adaptive call topology (mesh/elected-relay/SFU) instead of one fixed
  approach, in exchange for scaling from a 2-person DM call to a community
  voice room without either wasting server cost on small calls or
  collapsing under large ones.

## Spikes to run before this is fully load-bearing

This ADR is `Proposed`, not `Accepted`, in part because the following
should be validated with real prototypes — not vendor claims or web
research — before the team commits further engineering against these
choices:

1. **NAT traversal success-rate measurement.** Build a minimal
   iroh-based ping/connect tool, instrument it with telemetry, and run it
   across a diverse beta cohort (mobile carrier-grade NAT, symmetric home
   routers, corporate/campus networks, VPNs). Compare direct-connect % vs.
   relay-fallback % against both the ~90% (Tailscale-optimized) and ~70%
   (conservative academic baseline) figures cited above, so central-relay
   capacity planning is based on ANKAI's actual user network mix, not a
   borrowed number.
2. **WebRTC audio/E2EE prototype.** Validate webrtc-rs v0.20 (and/or
   str0m) delivers acceptable echo cancellation and noise suppression on
   ANKAI's target desktop OSes, and prove out frame-level E2E encryption
   through an SFU without breaking AEC or the jitter buffer. This is the
   highest-risk item in the whole ADR and should be spiked first.
3. **Group-call topology threshold.** Prototype both elected-peer-relay
   and always-SFU-above-N approaches to determine the actual participant
   count where mesh should hand off, and whether elected-peer-relay's
   added complexity is worth it versus simply routing to an SFU (community
   Node or central) above that threshold.
4. **Minimal "ANKAI Node" reference implementation.** Package
   `iroh-relay` + `coturn` (+ optional SFU worker), resource-capped, and
   pilot it with a small community to validate both the operator
   experience and whether content-addressed caching measurably reduces
   central relay/blob-serving load in practice.
5. **QUIC-datagram netplay prototype.** Wire one real emulator core's
   rollback netcode to QUIC unreliable datagrams over an iroh connection
   and measure latency/jitter/loss behavior before this becomes the
   committed netplay transport.

### Spike 1 status (NAT traversal success-rate measurement) — tool built, cohort measurement still pending, 2026-08-15

The minimal iroh-based ping/connect tool this spike calls for now exists at
`tools/nat-probe/`. It binds an iroh endpoint on the `N0` preset (real
DNS-based address lookup plus n0.computer's default relay servers — the
opposite of `core::p2p`'s deliberately relay-less `Minimal` preset, see
`tools/nat-probe/src/main.rs`'s module doc comment for why the two must stay
separate), takes on either the `listen` or `connect` role from the same
binary, and after connecting reports — via iroh's own `Connection::paths()`
introspection API, not a guess — whether the path actually selected for
application data is direct/hole-punched or relayed, alongside iroh's
per-path RTT estimate and a real application-level ping/echo RTT. Output is
plain structured `stdout` lines; there is no telemetry pipeline or
aggregation built (not needed for what this spike requires — see its own
doc comment for why).

**This does not close spike 1.** What's still outstanding is exactly what
the spike's own text asks for: running the tool "across a diverse beta
cohort (mobile carrier-grade NAT, symmetric home routers, corporate/campus
networks, VPNs)" and comparing the observed direct-connect vs.
relay-fallback split against the ~90% (Tailscale-optimized) and ~70%
(conservative academic baseline) figures cited above. That's real-world
data collection across networks nobody has run this tool on yet — an
agent working alone in one sitting cannot manufacture a beta cohort or
diverse NAT conditions, and no such data is claimed here. This should be
closed the same way ADR-0002's spikes 1 and 2 were: only after a human (or
several, across genuinely different networks) actually runs `nat-probe`
in the real world and reports back what it saw — see `PROGRESS.md` for
that precedent. Until then this line item stays open and this ADR remains
`Proposed`, not `Accepted`, on spike 1's account same as before.
