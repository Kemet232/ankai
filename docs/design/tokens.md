# ANKAI Design Tokens — "Liquid Y2K" v0

Status: Draft. These are the base tokens shared brand-wide; individual user
profiles/communities override a subset of them via the Theme SDK (see future
`docs/protocol/theme-spec.md`) — profile theming is a *constrained* override
of these tokens, not a free-for-all.

Design intent recap: Y2K/Aqua-era tactility + modern liquid glass + MySpace-
era personalization, rendered with native smooth animation. Not generic SaaS,
not flat corporate glassmorphism, not identical rounded cards everywhere.

## Color

Base palette (dark-first — ANKAI's default surface is a deep night sky, not
white):

| Token | Value | Use |
|---|---|---|
| `color.bg.void` | `#05070F` | app background base, deep blue-black |
| `color.bg.surface` | `#0D1122` | panel background under glass |
| `color.bg.surface-raised` | `#141A33` | elevated panel background |
| `color.glass.tint` | `#1B2340` @ 45% alpha | default glass material tint |
| `color.accent.cyan` | `#4CE8F0` | primary interactive accent |
| `color.accent.violet` | `#8B6CF0` | secondary accent, presence/activity |
| `color.accent.gold` | `#E8C468` | rare/premium accents only — achievements, marketplace highlights, verified marks. Used sparingly, never as a primary UI color. |
| `color.silver` | `#C7CEDB` | metallic surfaces, borders, iconography |
| `color.text.primary` | `#F2F4FA` | primary text on dark surfaces |
| `color.text.secondary` | `#A6ADC4` | secondary/meta text |
| `color.state.online` | `#4CE87B` | presence dot |
| `color.state.away` | `#E8C468` | presence dot |
| `color.state.busy` | `#F0605C` | presence dot |
| `color.state.offline` | `#5A6178` | presence dot |

Light mode exists (accessibility requirement) but is not the primary brand
expression — it inverts surfaces while keeping accent hues, at reduced glass
opacity (glass reads worse on light backgrounds; compensate with more solid
fills, not less contrast).

Profile/community theme overrides may replace `color.accent.*`,
`color.glass.tint`, and background imagery — never `color.text.*` contrast
ratios below WCAG AA, enforced by the theme validator at install time, not
left to creator discretion.

## Glass material

The core visual signature. A glass material token bundles:

| Property | Ultra | Normal | Low | Potato |
|---|---|---|---|---|
| `blur.radius` | 32px, layered | 20px | 8px | 0 (disabled) |
| `refraction` | on (edge distortion shader) | on (simplified) | off | off |
| `specular.highlight` | animated, GPU | static | static, reduced | off |
| `fallback.fill` | n/a | n/a | flat translucent (`glass.tint` @ fixed alpha) | flat opaque surface color |

Rule: **Potato mode must still be recognizably ANKAI** — same color tokens,
same layout, same iconography. Only the glass *rendering technique* degrades
(blur→flat tint, refraction→off), never the color language or information
density.

## Typography

- Display/headers: a geometric sans with subtle unconventional
  counters — evokes both Japanese UI precision and Y2K-futurist type without
  literally using a "techno" novelty font. (Exact family TBD in
  Phase 1 — needs a license check per `docs/adr` process; candidates to
  evaluate: a licensed geometric sans, e.g. something in the Sora / Space
  Grotesk family, or a commissioned/licensed custom face later.)
- Body: a high-legibility humanist sans, optimized for small sizes and dense
  UI (chat, forums) — not the same face as display type; the contrast
  between the two is part of the brand.
- Profile custom typography: creators pick from an **approved, sandboxed
  font set** shipped with the client — never arbitrary user-uploaded font
  files (font parsing is a real attack surface; also keeps rendering
  performant and consistent).
- Type scale: 12 / 14 / 16 / 20 / 24 / 32 / 40 / 56px, 1.25 modular-ish
  ratio, all user-scalable per the accessibility requirement (text scaling
  must not break glass panel layouts — design panels with flexible height).

## Motion

- Base easing: `cubic-bezier(0.16, 1, 0.3, 1)` (expo-out) for entrances,
  `cubic-bezier(0.7, 0, 0.84, 0)` for exits — snappy in, quick out, matches
  "liquid" feel without floatiness.
- Standard durations: micro `120ms`, standard `240ms`, panel-level `360ms`.
- `prefers-reduced-motion` / in-app "Reduced Motion" setting: replaces all
  transform/opacity choreography with instant or fade-only (150ms opacity),
  never fully removes state-change feedback (accessibility requires
  *something* signals the change, just not motion).
- Particle/ambient effects (profile backgrounds, etc.) are **additive only**
  — disabling them must never remove information, only decoration. Off by
  default in Potato mode; user-toggleable at Normal/Ultra regardless of
  performance tier for users who find them distracting.

## Elevation / depth

Rather than pure box-shadow elevation, depth is expressed through the glass
stack: `z0` void background → `z1` surface panels → `z2` raised/floating
panels (menus, modals) → `z3` always-on-top (notifications, active call
overlay). Each level increases blur radius and glass tint alpha slightly
rather than just adding shadow, reinforcing the "layered glass" read.

## Spacing

4px base unit: `space.1`=4, `space.2`=8, `space.3`=12, `space.4`=16,
`space.6`=24, `space.8`=32, `space.12`=48, `space.16`=64. Standard for any
native UI toolkit's layout system.

## Accessibility floors (non-negotiable regardless of theme)

- Text contrast ≥ WCAG AA against its actual rendered background (validated
  post-blur/tint, not just against the nominal token).
- All interactive elements keyboard-reachable with a visible focus ring
  (`color.accent.cyan` outline, 2px, never removed by themes).
- Reduced-transparency setting available independent of performance tier
  (some users want opaque panels for readability, not performance).
- Minimum tap/click target 32x32px effective area even inside dense chat/
  forum UI.

## Open items

- Exact type family selection + license (Phase 1).
- Full component spec (buttons, inputs, cards, module chrome) — write once
  the native UI toolkit is chosen (`docs/adr/0002`), since component
  structure depends on the toolkit's styling model.
