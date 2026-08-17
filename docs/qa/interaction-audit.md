# ANKAI interaction audit

Snapshot: 2026-08-17. This audit covers reachable desktop interaction
contracts; it does not replace runtime keyboard, screen-reader, resize, or
network-failure testing.

Run from the repository root:

```sh
scripts/check-interactions.sh
scripts/check-interactions.sh --strict
```

Normal mode fails deterministic broken contracts and reports UX gaps as
warnings. Strict mode promotes warnings to failures for release-candidate CI.

## Contracts checked

- Every callback exported by `AppWindow` has a matching Rust `app.on_*`
  registration.
- Top-level navigation labels and named route indexes are non-empty, unique,
  in range, and each index owns a reachable pane condition.
- Every actionable production `TouchArea` hands focus to a keyboard scope, and
  production files with custom controls expose accessible actions.
- Visible action/status strings do not advertise known placeholder destinations.
  Disabled local-only Hangout chat is accepted because its state and copy are
  truthful.
- Anime discovery, addon management, Letterboxd, Nyaa, and the embedded player
  expose loading, error, empty/unavailable, and retry states.
- The player shortcut contract is Space play/pause, Left/Right seek, Up/Down
  volume, M mute, F fullscreen, and Escape close menu, exit fullscreen, or close
  the player. Player buttons also support Enter/Space; timelines support arrow
  keys, with Home/End on seek.
- The global shortcut contract currently consists of Enter in the global search
  field. ANKAI does not currently promise an application-wide Ctrl/Cmd+K chord.
- The shell and data-heavy surfaces declare compact breakpoints; embedded player
  and Hangout surfaces share a 320 px minimum.

## Current automated result

The strict gate passes with zero failures and zero warnings. It verifies all 58
exported `AppWindow` callbacks, all seven named routes, custom pointer-to-keyboard
focus handoff, known-placeholder checks, async loading/error/empty/retry states,
player shortcuts, global-search submission, compact breakpoints, and player/
Hangout minimums.

The initial audit found five real gaps, all fixed in the same phase:

1. Home friends, discussions, and Hangouts now receive real ready/error state
   rather than hardcoded `"ready"` values.
2. Trending and popular anime use explicit request states, so a successful empty
   response no longer looks like perpetual loading.
3. Global search has a stable accessibility label.
4. `AppWindow` declares a supported 480×360 minimum and narrow content controls
   shrink or scroll at that boundary.
5. Home friend/discussion actions preserve the selected account or open the
   exact community instead of discarding row identity.

## Manual interaction matrix

1. Tab and Shift+Tab through every route at 960×640 and 560×360; activate every
   focused action with Enter and Space; confirm visible focus never disappears
   behind a ScrollView.
2. Repeat with VoiceOver and NVDA. Verify role, name, state, value, and selected
   status for navigation, catalog rows, player controls, sliders, and track menus.
3. For each network surface force loading, empty, timeout/error, retry success,
   and stale completion. Confirm focus remains stable and announcements are not
   duplicated.
4. Exercise all documented player shortcuts before and after clicking the video,
   while a menu is open, in fullscreen, when buffering, and on the error overlay.
5. Resize continuously through 320, 500, 560, 680, 720, 780, 900, and 920 px.
   Check clipping, scroll access, popup placement, and 200% text scaling.
