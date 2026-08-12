# 5. Emulation integration

Status: Proposed
Date: 2026-08-13

## Context

ANKAI wants game emulation as a first-class social feature: a unified game
library, one-click launch, rich presence ("playing Chrono Trigger, 3:14:02
played"), cloud/P2P save sync, achievements, netplay, and spectating —
folded into the same social graph as messaging, Top 8, and Hangouts.

This sits on top of a legally hazardous ecosystem. Two constraints are
non-negotiable for ANKAI as a company with a storefront and a 90/10 creator
marketplace (i.e. a commercial product, not a hobbyist GitHub repo):

1. **ANKAI must never distribute ROMs, ISOs, or copyrighted BIOS/firmware
   files.** These are copyrighted works owned by console manufacturers and
   publishers; only the user, having dumped their own legally-owned media,
   may supply them.
2. **ANKAI must not write emulator cores from scratch.** Cycle-accurate
   emulation of a game console is a multi-year, legally sensitive
   undertaking (see Nintendo v. Tropic Haze below) that duplicates work
   already done — and litigated — by existing projects. ANKAI's value-add
   is the social layer, not the emulation core.

The research below establishes what's safe to build on, what's radioactive,
and how the resulting architecture should be shaped.

### The libretro / RetroArch ecosystem

- **libretro API** (`libretro.h`): an open, MIT-licensed specification. A
  "core" is a shared library (`.so`/`.dll`/`.dylib`) implementing this API;
  a "frontend" loads cores and supplies video/audio/input/save drivers.
  Because the API itself is MIT, **anyone may write their own proprietary,
  commercial frontend** against it without inheriting GPL obligations from
  the API — this is explicitly the split libretro.org documents: the API is
  free to implement, RetroArch is just one (GPLv3) implementation of a
  frontend.
- **RetroArch**, the reference frontend, is licensed **GPLv3**. Linking
  against RetroArch's code, or forking/embedding it into ANKAI's binary,
  would pull ANKAI's client under GPLv3 obligations for that combined work.
- **Cores have per-project licenses, not a single "libretro license."**
  Per `docs.libretro.com/development/licenses/`:
  - GPL-family (commercial use allowed under GPL terms, but copyleft
    applies): Dolphin core (GPLv2), PPSSPP core (GPLv2), bsnes/higan
    (GPLv3), Mupen64Plus (GPLv3), Citra core (GPLv2).
  - MPL-2.0: mGBA.
  - **Explicitly non-commercial, source-available but *not* open source
    for our purposes:** Snes9x and Genesis Plus GX are both licensed
    "non-commercial" and their own license text says the software "may
    not be sold, nor may it be used in a commercial product or activity
    without the copyright holders' approval." MAME's licensing is mixed
    (BSD-3-Clause & GPLv2, with some drivers non-commercial).
  - **This is a hard blocker for bundling/redistributing those specific
    core binaries** inside a commercial product with a marketplace. It is
    *not* a blocker for a user downloading and running RetroArch (or those
    cores) themselves on their own machine — RetroArch's own core
    downloader handles this today and nobody has sued RetroArch Inc. over
    it, because RetroArch doesn't sell the cores or the frontend.

### Standalone emulators (non-libretro)

Several important systems are best served outside libretro, as standalone
projects with their own frontends and licenses:

- **Dolphin** (GameCube/Wii): GPLv2-or-later, actively maintained (2026
  stable releases ongoing). Nintendo forced Valve to block Dolphin's
  planned 2023 Steam release via a DMCA-adjacent legal threat; the Dolphin
  team abandoned that launch and continues to distribute only via its own
  site/GitHub. **Lesson: distributing through a storefront invites
  publisher legal attention even when the emulator itself is uncontroversially
  legal; direct/self-hosted distribution is the norm for this category.**
- **PCSX2** (PS2): GPLv3-or-later, actively maintained.
- **RPCS3** (PS3): actively maintained; notable because Sony still
  publicly hosts PS3 firmware, somewhat de-risking BIOS acquisition (still
  must be user-initiated, never bundled by ANKAI).
- **Ryujinx** (Switch) and **Citra/Yuzu** (3DS/Switch): **dead.** Nintendo
  sued Tropic Haze (Yuzu) in Feb 2024; the suit was framed around
  circumvention of Switch's encryption/DRM to enable piracy, not emulation
  per se, but it produced a $2.4M settlement, forced shutdown, source
  deletion, and domain surrender. Citra shut down in solidarity/shared
  infrastructure. Ryujinx followed shortly after under the same pressure.
  **This is the sharpest signal in the whole research pass: Nintendo will
  litigate emulation projects for its two most recent platforms, and the
  legal theory reached beyond "the emulator is illegal" into "the emulator
  facilitates circumvention of copy protection," which is a different (DMCA
  §1201) claim than the classic "emulators are legal, ROMs are the
  problem" doctrine.** ANKAI should treat **Switch and 3DS/post-3DS Nintendo
  platforms as out of scope indefinitely**, and should re-check the legal
  status of any emulator core before adding first-class support, not just
  once at launch.
- **snes9x standalone**, **mGBA standalone**, **PPSSPP standalone**: same
  per-project licenses as their core equivalents above.

### The standard legal posture of serious emulation frontends

RetroArch, Lutris, LaunchBox, ES-DE, and comparable projects converge on
the same three rules, which is the existing industry consensus ANKAI
should simply adopt rather than relitigate:

1. **Never bundle or distribute ROMs.** The user supplies their own dumps.
2. **Never bundle or distribute copyrighted BIOS/firmware.** Same rule —
   user supplies it, typically by dumping their own hardware. Some systems
   (PS3, some arcade boards) have legally-hostable or open-source
   replacement BIOS options; ANKAI should surface those where they exist
   but never ship proprietary BIOS files itself.
3. **No built-in ROM/BIOS acquisition from piracy sites.** No search
   integration, scraper, or "find this game" flow that points at ROM
   sites. Frontends only index files the user has already placed in a
   folder they designate.

ANKAI's library feature (scanning, metadata, boxart) must work exactly
like ES-DE/LaunchBox: point at a user-designated folder, hash/checksum
match against an open metadata database (e.g. No-Intro/Redump-style
checksums or the libretro/RetroArch database format) to identify what's
already there, and never write anything into that folder itself.

### Netplay

libretro/RetroArch's netplay is real, mature, and P2P by design (the docs
are explicit that P2P was chosen specifically to avoid running/paying for
relay servers for the common case): it works by buffering local and
remote input a few frames, using each core's built-in savestate
serialization to roll back and resimulate when input arrives late. It
requires a deterministic core and, ideally, direct connectivity between
peers. RetroArch also supports an optional relay/spectator role and a
lightweight in-app matchmaking directory, but the core protocol is
peer-to-peer.

**This is directly reusable**, not something ANKAI should reimplement:
netplay-capable libretro cores already solve frame-perfect P2P sync.
ANKAI's job is everything netplay does *not* do — friend discovery,
invites, session presence, and NAT traversal/signalling — which is exactly
the kind of thing ADR-0003 (P2P networking stack) is already scoped to
provide. The integration seam is: ANKAI's P2P layer does STUN/TURN-style
hole punching and exchanges IP:port + session parameters between two
already-authenticated ANKAI friends, then hands that off to RetroArch's
`--host`/`--connect` netplay CLI flags (or the equivalent for a standalone
emulator that supports netplay natively, e.g. some PCSX2/Dolphin builds
have experimental netplay too). ANKAI never touches the frame-sync
protocol itself.

### Achievements / rich presence (RetroAchievements)

- **rcheevos**, RetroAchievements' official C integration library, is
  **MIT-licensed**. It parses/evaluates achievement and rich-presence
  logic but does **not** make HTTP calls itself — the host application
  fetches achievement definitions and posts unlock events via
  RetroAchievements' HTTP API, and feeds the responses into rcheevos.
  This is clean to integrate directly into ANKAI (no GPL entanglement) if
  ANKAI wants native rich-presence strings ("Boss fight: Sephiroth") in
  its own UI rather than relying solely on RetroArch's built-in
  achievements menu.
- The mechanism for reading emulated game state without touching ROM
  data: RetroArch exposes a **local UDP Network Control Interface** (port
  55355 by default, opt-in via `network_cmd_enable`) with commands like
  `GET_STATUS` (returns loaded content's CRC32 + system, enough to
  identify "what game is this person playing" for presence) and
  `READ_CORE_RAM` (reads emulated RAM at achievement-defined addresses —
  the same mechanism RetroArch's own achievements/rich-presence system
  uses). ANKAI's presence service can poll this local, opt-in interface
  process-to-process; nothing here requires ANKAI to touch or copy any
  ROM/BIOS bytes.
- RAWeb (RetroAchievements' own backend/web platform) is GPLv3, but that's
  irrelevant to ANKAI — we'd only ever be an HTTP client of their public
  API plus the MIT rcheevos parsing library, never linking or forking
  RAWeb itself.

### Licensing/embedding boundary: shell out, don't link

FSF guidance on GPL is consistent on one axis regardless of "linking" vs.
"shelling out to a subprocess": the legal test is whether the result is
**one combined/derivative work** or **mere aggregation of independent
programs**. Two genuinely separate programs communicating over a process
boundary (CLI args, a local socket, stdin/stdout, or — as above — a UDP
control protocol) are the textbook case of mere aggregation and don't
propagate copyleft to the other program. In-process linking against
GPL(-only) code, or vendoring/forking GPL source into ANKAI's own binary,
is the textbook case of a combined work and would require ANKAI's client
to also be GPLv3 for that build — unacceptable for a commercial,
closed-source client.

This gives a clean, low-risk architecture:

## Decision

**ANKAI does not implement emulation. It orchestrates separately-installed,
unmodified, official emulator binaries as external processes, and builds
the social layer (library, presence, matchmaking, netplay signalling,
spectating, achievements) around them via each emulator's existing
IPC/CLI/network control surface.**

Concretely:

1. **Emulator acquisition**: ANKAI detects or prompts the user to install
   RetroArch (for the broad libretro-covered system catalog) and, where
   better served standalone, per-system emulators (Dolphin for
   GameCube/Wii, PCSX2 for PS2, RPCS3 for PS3, mGBA/PPSSPP standalone
   where preferred) via their own official installers/package managers —
   the same way Lutris and Playnite hand off to platform-native installers.
   ANKAI **never bundles emulator binaries or cores in its own installer or
   marketplace**, and never bundles BIOS files. This sidesteps the
   Snes9x/Genesis-Plus-GX non-commercial license problem entirely: ANKAI
   isn't distributing those cores, RetroArch's own (separately licensed,
   separately operated) core downloader is.
2. **Launching**: ANKAI shells out to the installed emulator binary as a
   subprocess (`retroarch -L <core> <content>` or the standalone
   emulator's CLI equivalent), passing the path to content the user has
   already placed in their library folder. No linking, no forking, no
   vendored emulator source in the ANKAI repo.
3. **Library**: ANKAI scans a user-designated folder, computes checksums,
   and matches against an open metadata DB (No-Intro/Redump-style
   hashes or the libretro database format) for boxart/title/platform
   metadata — never fetches or suggests ROM sources.
4. **Presence & rich presence**: ANKAI's client polls the running
   emulator's local control interface (RetroArch's UDP Network Control
   Interface `GET_STATUS`/`READ_CORE_RAM`, or the standalone emulator's
   equivalent scripting/RPC hook where available) and feeds results
   through the MIT-licensed `rcheevos` library (vendored directly into
   ANKAI, no licensing conflict) to produce "playing X, doing Y" presence
   strings for the social graph, and to post achievement unlocks to
   RetroAchievements' API on the user's behalf (opt-in, uses the user's
   own RetroAchievements account/credentials).
5. **Saves**: ANKAI treats save files/savestates the emulator already
   writes to disk as ordinary user data — sync them (encrypted, P2P or
   via ANKAI's thin cloud relay per the "thin cloud, fat client"
   philosophy) the same way it would sync any other user file. ANKAI does
   not need to understand save-file internals.
6. **Netplay**: ANKAI's P2P layer (ADR-0003) handles friend discovery,
   invites, and NAT traversal/signalling between two peers, then launches
   each peer's RetroArch (or netplay-capable standalone emulator) with the
   negotiated host/connect parameters. Frame sync, rollback, and
   determinism are entirely the emulator's problem, not ANKAI's.
7. **Spectating**: reuse RetroArch's existing netplay spectator role
   (join-as-spectator is already part of the libretro netplay protocol)
   fronted by ANKAI's presence/session UI, or, as a fallback for
   standalone emulators without a spectator mode, spectate via ANKAI's own
   screen-share/Hangouts pipe (already needed for watch parties) pointed
   at the host's emulator window.
8. **Scope boundary — platforms**: Switch, 3DS, and any actively-litigated
   platform are explicitly **out of scope** until/unless the legal
   landscape changes; re-verify status per-platform before adding support
   rather than trusting this ADR to stay current (Nintendo's DMCA-circumvention
   theory against Yuzu/Citra/Ryujinx shows this space moves fast).

### Licensing manifest entries required

Per ANKAI's third-party licensing review process, the following need
manifest entries once integration code lands (client-side dependencies
only — nothing here is vendored emulator/core source):

| Component | License | How ANKAI uses it | Notes |
|---|---|---|---|
| `rcheevos` | MIT | Vendored library, linked into ANKAI client | No conflict; MIT is permissive |
| `libretro.h` (API header only, if ANKAI writes any direct core-loading code later) | MIT | Reference/interface only | Do not bundle any actual core |
| RetroArch | GPLv3 | Launched as external subprocess only, never linked/forked/bundled | Must remain a separate install; document in threat-model/legal notes why this isn't a combined work |
| Dolphin / PCSX2 / RPCS3 / mGBA / etc. | GPLv2/GPLv3 (per-project) | Launched as external subprocess only | Same "mere aggregation" posture |
| Individual libretro cores (Snes9x, Genesis Plus GX, etc.) | Mixed, some non-commercial | Never bundled by ANKAI; user/RetroArch's own downloader manages these | Flag explicitly: ANKAI must never add a feature that downloads/bundles these itself |
| RetroAchievements API | Free API, ToS-governed | HTTP client only, user's own account | Confirm ToS allows third-party client apps before shipping |

## Consequences

**Easier:**
- No emulator-core engineering effort or ongoing accuracy/compatibility
  maintenance burden — ANKAI rides on the work (and legal precedent) of
  mature, actively-maintained upstream projects.
- Clean copyleft boundary: ANKAI's client can stay closed-source while
  still orchestrating GPL software, because the integration is
  process-boundary, not link-boundary.
- Netplay and achievements are "integrate an existing protocol," not
  "invent a new one" — lower risk, faster to ship.
- Matches the legal posture users, publishers, and the emulation community
  already recognize as legitimate (RetroArch/Lutris/ES-DE precedent),
  reducing both legal and reputational risk.

**Harder / traded away:**
- ANKAI depends on the user having (or being willing to install) a
  separate emulator; onboarding friction is real — the UX for
  "detect/prompt-install RetroArch or Dolphin" needs real design work.
- No control over emulator UX/performance/bugs; ANKAI can't easily add
  emulator-side features (e.g. a new save-state UI) without upstream
  cooperation — only what each emulator's control surface already exposes.
- Presence/rich-presence fidelity is capped by what RetroArch's Network
  Control Interface (or a standalone emulator's equivalent hook) exposes;
  standalone emulators vary widely in how much of this they expose, so
  presence quality will be uneven across systems (rich for
  libretro/RetroArch-backed systems, thinner for some standalones).
- Platform coverage is deliberately incomplete and will need continuous
  legal re-review (Switch/3DS excluded now; other platforms could become
  radioactive later the way Yuzu/Citra did).
- ANKAI must build and maintain a per-emulator "adapter" (CLI arg
  conventions, control-interface protocol, netplay flag differences) for
  each supported emulator rather than one uniform internal emulation API —
  more integration surface area than a single embedded core would need,
  traded deliberately for the licensing/legal safety of not embedding.

Sources consulted: libretro docs (docs.libretro.com — licenses, netplay,
network control interface), github.com/libretro/RetroArch, RetroAchievements
docs and github.com/RetroAchievements/rcheevos, Wikipedia (Dolphin, PCSX2,
Ryujinx, Yuzu), GameSpot/Engadget/Shacknews/KitGuru reporting on the
Yuzu/Citra Nintendo lawsuit and Dolphin's abandoned Steam release, GNU.org
GPL FAQ on mere aggregation vs. combined works.
