# 2. Native UI stack

Status: Accepted
Date: 2026-08-13

## Context

ANKAI's client is a genuinely native desktop app — no Electron, no Tauri-with-webview-as-primary-UI,
no WKWebView, no embedded-Chromium shell of any kind. The product spec calls this out explicitly
because the visual identity ("Liquid Y2K": translucent glass panels, blur/refraction, custom
typography, drag-and-drop rearrangeable profile modules, springy animation) is a core differentiator,
not a skin — it has to be built at the rendering-primitive level, not reskinned on top of OS-native
widgets (which rules out toolkits that hard-code a platform look, e.g. plain GTK/AppKit/WinUI
widget trees) and not faked with HTML/CSS in a browser engine (which rules out webviews).

Hard requirements the toolkit must satisfy:

1. **Genuinely native rendering.** No embedded browser engine anywhere in the primary UI path.
2. **Expressive, fully custom visual design.** Must support translucent/blurred "glass" materials,
   custom fonts, freeform layout, drag-and-drop reorderable modules, and smooth GPU-driven
   animation — not a toolkit that forces a generic native-widget look.
3. **Cross-platform now**: Windows, macOS, Linux, all three, at launch.
4. **Low resource footprint.** Must run acceptably on old dual-core CPUs, integrated graphics,
   ~4GB RAM. Fast cold start.
5. **GPU-accelerated custom rendering** for glass/blur/particle effects, with a "Potato mode"
   fallback that disables expensive effects for weak hardware.
6. **Accessibility**: screen readers, full keyboard navigation, reduced-motion, high-contrast,
   scalable text. This is a hard requirement, not an afterthought.
7. **Rust-core integration.** ANKAI's crypto/networking/business logic lives in a shared Rust
   `core` crate (see PROGRESS.md — "thin cloud, fat client"). The UI layer needs a low-friction
   path to call into it.
8. **Small-team maintainability.** ANKAI is built and maintained by a small team indefinitely.
   Ecosystem health, licensing (must be commercially usable and redistribution-friendly — no
   copyleft obligations that force us to open-source the client, no per-seat royalties), and
   how actively maintained a dependency is in 2026 all matter more here than for a large org
   that can absorb a toolkit's rough edges with headcount.
9. **Future mobile portability** is a nice-to-have. Desktop comes first and this must not be
   traded away to buy it.

All findings below are from live web research done 2026-08-13 (searches + fetches), not from
stale training data — toolkit maturity in this space moves fast enough that a year-old snapshot
would be misleading. Sources are linked inline and collected in **References**.

## Options considered

### Slint

**What it is**: a declarative Rust-first GUI toolkit (its own `.slint` markup DSL + Rust/C++/
JS/Python bindings), originally built for embedded/automotive, now explicitly positioned for
desktop too.

- **Rendering**: compiles to native code, no OS widget theming by default — renders everything
  itself via one of several backends (Skia, FemtoVG/OpenGL, software rasterizer). This is the
  toolkit's actual selling point: "native" means "compiled native binary drawing its own pixels,"
  not "wraps OS widgets."
- **Styling for Liquid Y2K**: fully custom by design — you are never fighting a native widget
  theme. Gaps as of 2026: no first-class blur/backdrop-filter primitive yet; there's an open
  feature request for custom shader support ([slint-ui/slint#10887](https://github.com/slint-ui/slint/issues/10887))
  and only a community workaround for a translucent blur backdrop on macOS
  ([discussion #5710](https://github.com/slint-ui/slint/discussions/5710)). Drag-and-drop for
  reorderable modules is workable today: there's a working Kanban-board example doing card
  reordering, plus experimental `DragArea`/`DropArea` elements in progress.
- **GPU acceleration**: yes, via the Skia or FemtoVG(OpenGL) renderer backends; a software
  renderer exists too, which is a natural "Potato mode" fallback path.
- **Footprint**: this is Slint's strongest point — it was built to run on microcontrollers with
  kilobytes of RAM, so a desktop build is comfortably lean; startup is fast (no VM, no JIT).
- **Accessibility**: integrates [AccessKit](https://accesskit.dev/) (the shared Rust
  accessibility-tree crate also used by egui, Xilem, Freya, Bevy). Real gaps exist as of 2026:
  text-input widgets have known screen-reader issues on Windows (garbled cursor/braille tracking,
  missing properties — [#8732](https://github.com/slint-ui/slint/issues/8732),
  [#2895](https://github.com/slint-ui/slint/issues/2895)), and enabling the accessibility feature
  has a measurable performance cost on large list views
  ([#3867](https://github.com/slint-ui/slint/issues/3867)). Basic screen-reader/keyboard nav for
  non-text-input widgets works; text fields need validation before we can call this "done."
- **License**: triple-licensed — GPLv3 (open source only), a **royalty-free license** that
  explicitly permits proprietary desktop/mobile/web apps at no cost (excludes embedded, which we
  don't need), or a paid commercial license with support. The royalty-free tier is exactly what
  ANKAI needs: free, proprietary-compatible, no redistribution tax. ([Slint 1.1 announcement](https://slint.dev/blog/slint-1.1-released), [license text](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md))
- **Maintenance**: backed by a funded company (SixtyFPS GmbH / Slint), dual-licensing is their
  actual business model so they have a commercial incentive to keep it healthy, 50+ external
  contributors, regular releases (1.6 shipped in 2026 with accessibility and design-mode
  improvements — [blog](https://slint.dev/blog/slint-1.6-released)). Lowest key-person-risk of
  the pure-Rust-native options evaluated here.
- **Known limitations**: no built-in blur/shader system yet (biggest gap vs. the Liquid Y2K
  bar); DSL means the team learns a second (small) language alongside Rust, though logic still
  lives in Rust; mobile support (Android/iOS) is present but explicitly less mature than desktop.

### GPUI (Zed's UI framework)

**What it is**: the GPU-accelerated, hybrid immediate/retained-mode UI framework built by Zed
Industries to power the [Zed](https://zed.dev/) editor, published as a standalone crate on
crates.io with the stated goal ("today it's Zed's, tomorrow it's yours") of becoming a general
framework.

- **Rendering**: draws everything itself on the GPU; Zed itself is proof this architecture can
  deliver a smooth, heavily custom, animated native UI at high frame rates — probably the closest
  existing example of "what Liquid Y2K should feel like" of anything surveyed here.
- **Styling for Liquid Y2K**: excellent ceiling — Zed's own UI already does gradients, blur-like
  effects, smooth 120fps animation, custom typography. If the visual bar is the only axis, this
  is arguably the strongest candidate.
- **GPU acceleration**: yes, core to the design.
- **Footprint**: not extensively documented independent of Zed, but Zed itself is known for fast
  startup and modest memory use relative to Electron-based editors — a reasonable proxy.
- **Accessibility**: this is the disqualifying gap today. Zed on Windows is reported as
  completely silent to screen readers as of a 2025/2026 issue — testers using JAWS and NVDA got
  no output at all ([zed-industries/zed#41138](https://github.com/zed-industries/zed/issues/41138)).
  The root cause is architectural: GPUI was built from scratch and doesn't yet expose an
  accessibility tree the way AccessKit-integrated toolkits do; Zed's own maintainers describe
  accessibility as "a long project, likely lasting far beyond 1.0"
  ([discussion #6576](https://github.com/zed-industries/zed/discussions/6576)). Given ANKAI's
  accessibility requirement is explicit and hard, this alone rules GPUI out for now.
- **License**: declared Apache-2.0, but as of a May 2026 issue, a *default release build
  statically links GPL-3.0-or-later code* transitively (`gpui → sum_tree → ztracing`), meaning
  binaries depending on gpui may currently inherit GPL-3.0 source-availability obligations —
  directly contrary to ANKAI's "commercially usable, redistribution-friendly" requirement
  ([zed-industries/zed#55470](https://github.com/zed-industries/zed/issues/55470)). Unresolved
  as of this research date.
- **Maintenance**: effectively single-vendor (Zed Industries) driven; some third-party adoption
  exists (e.g. [longbridge/gpui-component](https://github.com/longbridge/gpui-component)) but the
  framework is explicitly pre-1.0, not yet documented or API-stabilized for outside consumption,
  and subject to churn as Zed's own needs evolve.
- **Verdict**: technically the most exciting rendering story, but not adoptable today — no
  accessibility story, an open license-contamination question, and a not-yet-standalone API.
  Worth re-evaluating in 12–18 months if AccessKit lands and the license issue is resolved.

### Qt / QML via cxx-qt

**What it is**: Qt Quick (QML) as the UI layer, safely bridged to Rust via
[cxx-qt](https://github.com/KDAB/cxx-qt), maintained by KDAB (a long-established, respected Qt
consultancy). The alternative binding, `qmetaobject-rs`, is comparatively stagnant and doesn't
support QWidgets — cxx-qt is the credible option in this family.

- **Rendering**: Qt Quick's scene graph renders via its own RHI (Vulkan/Metal/D3D12/OpenGL), not
  native OS widgets — this is genuinely native, GPU-composited rendering.
- **Styling for Liquid Y2K**: the strongest of any candidate here. QML ships built-in
  `MultiEffect`/`GraphicalEffects`-style blur, opacity masks, gradients, and arbitrary
  `ShaderEffect` (raw GLSL/HLSL/MSL via Qt's shader pipeline) out of the box. "Frosted glass"
  QML UIs are a well-trodden pattern with years of examples. Drag-and-drop reordering is a solved
  problem in QML.
- **GPU acceleration**: yes, mature, cross-platform, battle-tested for decades.
- **Footprint**: heavier baseline than the pure-Rust minimal toolkits (Qt Quick apps typically
  run tens of MB RAM at idle rather than single-digit MB) but still dramatically lighter than
  Electron/Chromium, and Qt has a long track record on genuinely low-end/embedded hardware.
- **Accessibility**: the most mature option evaluated, full stop. Qt has shipped real,
  OS-integrated screen reader support (NVDA/JAWS/VoiceOver/Orca), keyboard nav, high-contrast,
  and text scaling for many years — this is a solved problem here, not a work-in-progress.
- **License**: LGPLv3 (free, commercially usable, redistribution-friendly as long as we dynamically
  link and preserve users' re-linking rights — standard practice, does not require open-sourcing
  ANKAI) or a paid commercial license if we ever want static linking/extra flexibility. Workable,
  but it's a licensing model the team has to actively manage (vs. Slint's simpler "free
  royalty-free tier, done").
- **Maintenance**: cxx-qt is actively maintained by KDAB with recent activity and Qt-conference
  presence through 2026 ([kdab.com/cxx-qt](https://www.kdab.com/cxx-qt/)); Qt itself is one of the
  longest-lived, best-funded UI toolkits in existence.
- **Known limitations**: brings in a full C++ toolchain and Qt's build system alongside Cargo —
  meaningfully more CI/build complexity than a pure-Rust dependency graph. QML is a second UI
  language the (small) team has to learn and maintain proficiency in, on top of Rust. This is the
  main cost against "small-team maintainability."

### egui

**What it is**: the most popular pure-Rust immediate-mode GUI library (single primary maintainer,
[emilk](https://github.com/emilk/egui), large contributor base, co-stewarded in parts by
[rerun.io](https://www.rerun.io/)).

- **Rendering**: draws itself via wgpu or glow backends, GPU-accelerated, genuinely native.
- **Styling for Liquid Y2K**: workable but not the natural fit. Custom shaders/blur are possible
  via `PaintCallback`/`egui_wgpu::Callback`, and a working "blurry glass window" implementation
  exists in the wild ([mxs.dev writeup](https://www.mxs.dev/blog/egui-wgpu-blurry-windows)) proving
  it's achievable. Drag-and-drop reordering is a solved, actively-maintained problem via
  [`egui_dnd`](https://crates.io/crates/egui_dnd) (co-owned by rerun.io). The structural issue is
  immediate-mode's single-pass layout model: it's genuinely weaker at complex, responsive,
  deeply-nested animated layouts (the kind ANKAI's rearrangeable profile-module grid needs) than
  retained-mode toolkits, and its default look/ergonomics skew toward developer-tool UIs, not
  heavily branded consumer apps. A documented real-world account of a team abandoning
  egui/Iced/Slint/GTK for a custom-styled data-heavy desktop tool, in favor of Electron, over
  "numerous small but critical shortcomings" is a useful cautionary data point (title alone is
  telling: ["Rust GUI Framework Benchmark: egui vs Iced vs Slint vs GTK — Why I Chose Electron"](https://medium.com/@build_break_learn/rust-gui-framework-benchmark-egui-iced-slint-gtk-electron-d88596c042fb)).
- **GPU acceleration**: yes.
- **Footprint**: excellent — minimal, fast startup, no separate runtime.
- **Accessibility**: solid relative to other Rust-native options — AccessKit is built in and
  enabled by default in `eframe`, implementing native APIs on Windows/macOS, plus an experimental
  built-in screen reader for platforms AccessKit doesn't cover yet.
- **License**: dual MIT/Apache-2.0 — ideal, no concerns.
- **Maintenance**: very active, large community, huge crates.io usage; the main risk is that core
  maintainership is concentrated in one person, though the ecosystem (rerun.io, contributors) has
  broadened this somewhat.
- **Verdict**: best fit here for internal tools, and a safe fallback if Slint's blur/glass spike
  fails, but not the first choice for a flagship, heavily art-directed consumer social app.

### Flutter with a Rust core via FFI

**What it is**: Flutter as the UI/rendering layer, with ANKAI's Rust `core` crate exposed via
[flutter_rust_bridge](https://github.com/topics/flutter-rust-bridge).

- **Rendering — is it honestly "no webview"?** Yes. Flutter does not embed a browser engine; it
  draws every pixel itself via its own engine (Skia historically, now increasingly
  [Impeller](https://dev.to/eira-wexford/how-impeller-is-transforming-flutter-ui-rendering-in-2026-3dpd),
  a renderer purpose-built for Flutter, targeting Metal/Vulkan/D3D directly). This is a genuinely
  different category from Electron/Tauri-webview/WKWebView and satisfies the requirement.
- **Styling for Liquid Y2K**: excellent — Flutter's whole design philosophy is pixel-perfect
  custom branding, and `BackdropFilter` gives real, built-in blur/glassmorphism with no custom
  shader work required. Animation is a first-class, well-documented citizen. This is arguably the
  easiest path to the exact aesthetic ANKAI wants, out of every option evaluated.
- **GPU acceleration**: yes, mature.
- **Accessibility**: the most mature semantic-tree/screen-reader/high-contrast/text-scaling
  support of any option surveyed apart from Qt — a real first-class subsystem, not
  bolted-on.
- **Footprint**: the weak point against ANKAI's requirements. Flutter desktop carries a Dart
  runtime + engine baseline that is heavier than Slint/egui/GPUI (tens to 100+MB RAM at idle is
  typical), workable but not comfortably "old dual-core, 4GB RAM, potato mode" lean without real
  tuning effort.
- **Rust integration**: requires a generated FFI bridge (flutter_rust_bridge) rather than being
  Rust natively — every new Rust API surface needs bridge codegen maintained, and Dart becomes a
  second primary application language, not just a UI DSL. This cuts against "thin, small,
  Rust-core-first team" more than any other candidate's integration story.
- **License**: BSD-3-Clause, fully permissive, Google-backed, enormous ecosystem and hiring pool.
- **Mobile portability**: this is Flutter's best-in-class strength — trivially the strongest
  future-mobile story of any option here, if that ever becomes a priority.
- **Verdict**: a legitimate, honest "no webview" option with the best accessibility+visual-design
  combination after Qt, but the two-language (Dart+Rust) maintenance burden and heavier footprint
  are real costs against a small Rust-first team building for weak hardware. Kept as a documented
  fallback, not the pick.

### Dioxus — native renderer mode (Blitz + Vello/wgpu), not the webview mode

**Important nuance**: Dioxus's default, production-recommended "desktop" target uses a system
webview (Tao/Wry) — that mode is explicitly **excluded** by ANKAI's requirements, same as Tauri.
The relevant candidate is specifically **Dioxus Native**, the experimental renderer that replaces
the webview with [Blitz](https://github.com/DioxusLabs/blitz) (a modular HTML/CSS layout engine)
driving [Vello](https://github.com/linebender/vello) (GPU 2D rendering) over wgpu.

- **Rendering**: genuinely native, GPU-accelerated, no browser engine — components → Blitz layout
  → Vello → wgpu → Vulkan/Metal/DX12.
- **Styling for Liquid Y2K**: potentially the most expressive path of the pure-Rust options,
  because styling is real CSS (via Blitz) rather than a bespoke DSL — CSS is a well-understood,
  powerful tool for exactly this kind of layered, animated, glass-like design. Backdrop-filter/blur
  support specifically within Blitz was not confirmed mature at time of research.
- **Maturity**: explicitly experimental and under rapid development; it is not the officially
  recommended production path today, and the API/feature surface is expected to keep moving.
  Betting ANKAI's primary UI on it now would mean building on a moving foundation.
- **License**: MIT, all-Rust (no second language, no FFI bridge — Dioxus components and the Rust
  `core` crate can live in the same workspace), which is otherwise exactly the shape ANKAI wants.
- **Verdict**: the option most worth re-evaluating in a future ADR revision (6–12 months out) as
  it matures — genuinely promising, not yet safe to build a flagship app on.

### Honorable mentions (briefly)

- **Iced** (used by [System76's COSMIC desktop](https://en.wikipedia.org/wiki/COSMIC_desktop)):
  pure-Rust, Elm-architecture, wgpu-backed, MIT-licensed. Notable because COSMIC 1.3 shipped a
  real, shader-based "frosted glass" desktop-wide effect in 2026 using a dual-Kawase blur for
  performance ([AlternativeTo](https://alternativeto.net/news/2026/7/cosmic-desktop-1-3-adds-frosted-glass-style-and-gpu-power-monitoring/),
  [Phoronix](https://www.phoronix.com/news/COSMIC-Frosted-Glass)) — solid proof that Rust+wgpu can
  hit the Liquid Y2K bar at OS-compositor scale with acceptable performance. Backed by System76
  (a real company shipping it in production), governance concentrated around one lead maintainer
  historically but with growing outside adoption. A credible dark-horse alternative to Slint;
  didn't get the nod mainly because its styling model (Elm-style, code-driven) is a bigger
  departure from a designer-friendly workflow than Slint's markup DSL or QML, and its AccessKit
  integration is less far along than Slint's or egui's as of this research.
- **Makepad**: shader-first, MIT-licensed, live-coding DSL, actively developed; interesting for
  its shader-native styling model (glass/blur would be very natural here) but ecosystem/community
  size and accessibility story are both far behind Slint/Qt/egui as of 2026.
- **Xilem** (Linebender, uses Vello + AccessKit): the most architecturally ambitious pure-Rust
  reactive UI framework, explicitly alpha-stage as of 2026. Worth watching, not betting on yet.
- **Floem** (from the Lapce editor team) and **Freya** (Skia-based, MIT, evolved out of Dioxus):
  both real, both used in production by their own flagship apps (Lapce, respectively), both too
  small/early in community size and accessibility maturity to be a primary pick for a team that
  isn't also going to be a co-maintainer of the framework itself.

## Comparison summary

| Toolkit | No webview | Glass/blur ceiling | Footprint | Accessibility (2026) | License | Rust-native | Maintenance risk |
|---|---|---|---|---|---|---|---|
| **Slint** | Yes | Good, custom-shader gap open | Excellent (embedded-grade) | Partial (text-field gaps) | Royalty-free tier — good | Yes | Low (funded co.) |
| GPUI | Yes | Excellent (proven by Zed) | Good (unverified standalone) | **None today** | Apache-2.0 w/ open GPL-contamination bug | Yes | Medium-high (pre-1.0, single-vendor) |
| Qt/QML (cxx-qt) | Yes | Excellent (mature, built-in) | Moderate | **Excellent** | LGPLv3/commercial — manageable | No (C++/QML + bridge) | Low (KDAB + Qt) |
| egui | Yes | Good w/ effort | Excellent | Good | MIT/Apache-2.0 — ideal | Yes | Low-medium (concentrated maintainership) |
| Flutter+Rust FFI | Yes (honestly) | Excellent, built-in | Moderate-heavy | **Excellent** | BSD-3 — ideal | No (Dart + FFI bridge) | Low (Google-backed) |
| Dioxus Native | Yes | Good (real CSS) potentially | Unknown | Unknown/early | MIT — ideal | Yes | Medium-high (experimental) |
| Iced/COSMIC | Yes | Excellent (proven by COSMIC) | Good | Behind Slint/egui | MIT — ideal | Yes | Low-medium |

## Decision

**Adopt Slint as the primary UI toolkit for the ANKAI desktop client**, using its Rust API
(not the C++/JS/Python bindings) under the **royalty-free license tier**, with the **Skia
rendering backend** for full-effects mode and the **software/FemtoVG backend** as the basis for
Potato mode.

Rationale, mapped directly to the requirements:

- **No webview**: satisfied — Slint compiles to native code and draws its own pixels; there is no
  embedded browser engine anywhere in the stack.
- **Liquid Y2K visual bar**: Slint's whole premise is a fully custom look, not themed OS widgets,
  which is the right starting posture. The one real gap — no first-class blur/shader primitive —
  is judged closeable via a spike (see below) using the Skia backend's canvas access, the same way
  the egui community closed an equivalent gap with wgpu paint callbacks. This is flagged as the
  single highest-risk item in this decision and is not being hand-waved.
- **Cross-platform now**: Windows/macOS/Linux all first-class, today.
- **Low RAM/CPU, old hardware, fast startup**: this is Slint's best-differentiated strength among
  every candidate evaluated — it was designed for microcontrollers, so a desktop build has
  enormous headroom on a "4GB RAM, integrated graphics, dual-core" target machine.
- **GPU acceleration + Potato mode**: the Skia/FemtoVG backend gives GPU acceleration for the full
  glass aesthetic; the software rasterizer backend is a natural, already-existing fallback path
  for Potato mode rather than something we'd have to build from scratch.
- **Accessibility**: AccessKit integration exists and covers the baseline (keyboard nav, screen
  reader tree for standard widgets). It is **not** fully solved — text-input screen-reader
  behavior on Windows has open bugs. This is the second flagged risk item and needs validation
  (and possibly upstream contribution) before Phase 1 ships anything with real text-input-heavy
  screens (messaging, profile editing) to users who depend on assistive tech.
- **Rust-core integration**: native — Slint's Rust API composes directly with ANKAI's `core`
  crate in the same Cargo workspace, no FFI/bridge layer, no second business-logic language.
- **Small-team maintainability, licensing, 2026 activity**: best combination of the field — a
  funded company with a direct commercial incentive to keep the project healthy, a simple and
  genuinely free royalty-free license tier with no dynamic-linking/redistribution gymnastics
  (unlike Qt/LGPL) and no open license-contamination bug (unlike GPUI), pure Rust (no second UI
  language, unlike Qt/QML or Flutter/Dart), and a small but real DSL to learn (`.slint` markup)
  that's considerably smaller a lift than QML or a whole second runtime.
- **Future mobile**: Slint already has experimental Android/iOS targets — not a strong story yet,
  but a real one, unlike GPUI or egui which are realistically desktop-only today.

**Documented fallback**: if the blur/glass spike below fails to reach an acceptable visual and
performance bar, the fallback is **Qt/QML via cxx-qt**. It has the most mature answers to both of
Slint's flagged risks (blur/shader effects are built-in and battle-tested; accessibility is fully
solved), at the cost of a second UI language and a heavier, C++-toolchain-dependent build. This is
a real, workable fallback, not a hypothetical one — it should not require restarting the ADR
process, just superseding this decision with the evidence from the failed spike.

**Not adopted, with explicit reasoning to avoid relitigating without new information**:
- **GPUI** — no accessibility story and an unresolved license-contamination bug today; revisit if
  both are resolved and it stabilizes past 1.0.
- **egui** — immediate-mode layout model is a structural mismatch for ANKAI's complex, deeply
  custom, animated module-grid UI; kept as the fallback-of-the-fallback for any individual
  internal/admin tool screens where dev velocity matters more than visual polish.
- **Flutter+Rust FFI** — an honest, capable option, but the two-language (Dart+Rust) surface and
  heavier resource footprint work against the small-team and old-hardware requirements more than
  Slint or Qt do.
- **Dioxus Native** — too early/experimental for a production commitment today; the best
  candidate to re-evaluate first in any future revision of this ADR, given it's pure Rust and uses
  real CSS for styling.

## Consequences

**Easier**:
- Business logic in `core` and UI code in `client` share one language, one toolchain, one CI
  pipeline — no FFI bridge to maintain or debug across a language boundary.
- Resource-footprint and startup-time requirements are very likely satisfied by default, without
  ongoing performance-tuning effort, freeing engineering time for product work instead of fighting
  the runtime.
- Onboarding new contributors is lower-friction than a Qt/QML or Flutter/Dart stack: one language
  (Rust) plus one small declarative DSL, rather than a second full programming ecosystem.
- Licensing is simple to reason about and explain to anyone doing due diligence (investors,
  acquirers, auditors): free, proprietary-compatible, no royalties, no copyleft obligation.

**Harder**:
- Custom shader/glass/particle effects are not out-of-the-box; we are committing to building and
  maintaining bespoke rendering code (likely via Skia canvas access or a custom compositing layer)
  for the effects that are the product's visual signature. This is real, ongoing engineering
  investment, not a one-time cost — every new "glass" surface in the design system needs someone
  who understands this layer.
- Accessibility for text-heavy screens (DMs, profile bios, community posts) needs active
  validation and possibly upstream contribution to Slint/AccessKit rather than "it just works" —
  budget real QA time with actual screen readers (NVDA, JAWS, VoiceOver, Orca) before those
  surfaces ship, not after.
- Hiring: the pool of engineers who already know Slint is far smaller than Qt, Flutter, or web
  developers generally. We are trading a larger hiring pool for a smaller, more Rust-aligned one —
  consistent with the project's overall Rust-first culture, but a real constraint to plan around.
- Mobile portability, if it ever becomes a real priority, will take more work than it would have
  under Flutter — Slint's mobile targets are real but clearly less mature than its desktop story.
  Accepted per the requirement that desktop-first must not be traded away for mobile-readiness.
- If the blur/glass spike fails and we fall back to Qt/QML, we lose the "one language" property
  and take on a second UI language plus a heavier C++ build/CI surface. This is a real switching
  cost, which is exactly why the spike should happen early (Phase 1, before deep investment in
  Slint-specific UI code) rather than being discovered late.

## Spike / prototype plan to de-risk before further engineering

Before committing further engineering to Slint (i.e., before Phase 1's native shell work goes
much beyond a window + navigation skeleton), run two time-boxed spikes:

1. **Glass/blur rendering spike** (highest-priority risk). Build a single prototype screen (e.g.
   a mock profile card with a translucent, blurred glass panel over a moving/colorful background,
   plus one drag-and-drop reorderable module list) using Slint's Skia backend. Confirm: (a) an
   acceptable visual result is achievable without forking Slint or waiting on upstream custom-
   shader support, (b) frame time stays acceptable on a low-end reference machine (old dual-core,
   integrated GPU, 4GB RAM — pick and document one specific real or virtualized target), and (c)
   a "Potato mode" toggle that swaps to the software/FemtoVG backend (or simply disables the blur
   pass) measurably recovers performance headroom. If (a) or (b) fail, invoke the documented
   fallback to Qt/QML rather than sinking further time into workarounds.

2. **Accessibility validation spike.** Build one representative text-input-heavy screen (e.g. a
   DM compose box or profile-bio editor) and manually test it with NVDA and a Windows screen
   reader, VoiceOver on macOS, and Orca on Linux. Document exactly which of the known upstream
   gaps (Windows text-field issues in particular) reproduce, and decide per-gap whether to
   upstream a fix, work around it locally, or accept the risk for v1 with a tracked follow-up.

Both spikes should be small, throwaway prototypes — days, not weeks — and their outcomes should
be recorded as an update to this ADR (or a superseding ADR if the fallback is triggered) before
Phase 1 native-shell work is considered complete.

### Spike 1 results (glass/blur rendering) — closed, 2026-08-13

Prototype implemented: `client/` crate (Slint 1.17, `renderer-skia` feature), UI in
`client/ui/app.slint` + `client/ui/module-card.slint`, entry point `client/src/main.rs`.
Covers all three elements this spike asked for: a translucent glass panel over a
colorful/patterned moving background, a drag-and-drop reorderable module list (5 mock
profile-module cards, TouchArea-based reorder — Slint 1.17 has no stable DragArea/DropArea),
and a Potato-mode toggle that swaps the glass panel to an opaque flat fill and drops the
shadow blur.

What's confirmed:

- Builds clean on this dev machine: `cargo build/clippy --workspace --all-targets` — zero
  warnings. `cargo fmt --check` passes.
- Runs and renders correctly: manually launched (`cargo run -p client`) and visually
  inspected on this dev machine (Apple Silicon Mac, not the low-end reference target).
  Glass panel (tint + specular strip + drop-shadow), confetti background, and the
  reorderable card list all render as designed; Potato-mode toggle visibly swaps the panel
  to the flat/opaque fill.
- Confirms the honest ceiling this ADR anticipated: Slint 1.17 has no backdrop-filter /
  blur-behind primitive (slint-ui/slint#2066, #10887 still open), so this is alpha-blended
  tint + border + specular gradient + drop-shadow, **not** true dynamic Gaussian blur of the
  content behind the panel. Documented inline in `app.slint`.

**Low-end hardware validation (criteria b/c):** confirmed successful by the human on an old
dual-core machine with integrated GPU and 4GB RAM — matching the reference profile this ADR
originally called for. Reported as successful directly by the user (2026-08-13); exact
measured frame-time numbers and the Potato-mode headroom delta were not separately logged
here — if precise figures are needed later (e.g. to set a numeric performance budget), ask
the user or re-run the same prototype with profiling instrumentation.

**Read:** all three spike-1 criteria are now satisfied — (a) acceptable visual result without
forking Slint, (b) acceptable frame time on the low-end reference profile, (c) Potato mode
recovers headroom on that same hardware. Spike 1 is closed; the Qt/QML fallback documented
elsewhere in this ADR is not triggered.

### Spike 2 results (accessibility validation) — partial, 2026-08-13

The human reported "accessibility passed." Follow-up established what was actually tested:
VoiceOver on macOS, against the existing nav skeleton (`client/ui/app.slint` — sidebar +
plain labels, no text-entry fields).

This does **not** close the spike as scoped above:

- No text-input-heavy screen exists in this repo yet (no DM compose box, no profile-bio
  editor, nothing with a text-entry field) — the spike explicitly calls for testing one,
  since text fields are where this ADR's specifically-flagged risk lives.
- The one platform/reader combo actually tested (VoiceOver/macOS) is not the one this ADR
  calls out as highest-risk — that's NVDA/JAWS on Windows (slint-ui/slint#8732, #2895,
  "completely silent to screen readers" per this ADR's Context section). Windows was not
  tested. Orca on Linux was not tested either.

**Read:** treat this as "a plain, non-text-input Slint screen is VoiceOver-navigable on
macOS" — a genuinely useful data point, but not a substitute for the spike's actual target.
**Still open:** build a real text-input screen and test it with NVDA/JAWS on Windows in
particular, plus Orca on Linux, before treating Slint's accessibility risk as retired. Don't
let this partial result create false confidence that the gate has cleared — see PROGRESS.md.

## References

- Slint: [FAQ](https://github.com/slint-ui/slint/blob/master/FAQ.md), [1.1 royalty-free license announcement](https://slint.dev/blog/slint-1.1-released), [1.6 release notes](https://slint.dev/blog/slint-1.6-released), [funding/hiring post](https://slint.dev/blog/slint-funding-and-hiring), custom shader request [#10887](https://github.com/slint-ui/slint/issues/10887), macOS blur discussion [#5710](https://github.com/slint-ui/slint/discussions/5710), Windows text-field a11y issues [#8732](https://github.com/slint-ui/slint/issues/8732) / [#2895](https://github.com/slint-ui/slint/issues/2895), a11y perf issue [#3867](https://github.com/slint-ui/slint/issues/3867).
- GPUI: [crates.io](https://crates.io/crates/gpui), [gpui.rs](https://www.gpui.rs/), Windows screen-reader silence [#41138](https://github.com/zed-industries/zed/issues/41138), a11y roadmap discussion [#6576](https://github.com/zed-industries/zed/discussions/6576), license contamination [#55470](https://github.com/zed-industries/zed/issues/55470).
- Qt/cxx-qt: [KDAB cxx-qt](https://www.kdab.com/cxx-qt/), [cxx-qt repo](https://github.com/KDAB/cxx-qt).
- egui: [repo](https://github.com/emilk/egui), [egui_dnd](https://crates.io/crates/egui_dnd), [blurry-windows writeup](https://www.mxs.dev/blog/egui-wgpu-blurry-windows), cautionary account: ["why I chose Electron"](https://medium.com/@build_break_learn/rust-gui-framework-benchmark-egui-iced-slint-gtk-electron-d88596c042fb).
- Flutter+Rust: [Impeller in 2026](https://dev.to/eira-wexford/how-impeller-is-transforming-flutter-ui-rendering-in-2026-3dpd), [flutter_rust_bridge topic](https://github.com/topics/flutter-rust-bridge).
- Dioxus Native/Blitz: [Blitz repo](https://github.com/DioxusLabs/blitz), [platform renderers](https://deepwiki.com/DioxusLabs/dioxus/5-platform-renderers).
- Iced/COSMIC: [COSMIC 1.3 frosted glass](https://alternativeto.net/news/2026/7/cosmic-desktop-1-3-adds-frosted-glass-style-and-gpu-power-monitoring/), [Phoronix coverage](https://www.phoronix.com/news/COSMIC-Frosted-Glass), [Iced 0.14 release](https://github.com/iced-rs/iced/releases/tag/0.14.0).
- AccessKit: [accesskit.dev](https://accesskit.dev/), [adopter list](https://github.com/AccessKit).
- General landscape: [Are We GUI Yet](https://areweguiyet.com/).
