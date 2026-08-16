#!/usr/bin/env bash
# Repeatable, non-mutating release checks for ANKAI's reachable desktop UI.
#
# Default mode is fast and suitable for every UI change. `--full` also runs
# Rust compilation/tests. `--strict` promotes readiness warnings to failures.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || exit 1

run_full=0
strict=0
for argument in "$@"; do
    case "$argument" in
        --full) run_full=1 ;;
        --strict) strict=1 ;;
        *)
            echo "usage: $0 [--full] [--strict]" >&2
            exit 2
            ;;
    esac
done

failures=0
warnings=0

pass() {
    echo "PASS  $1"
}

fail() {
    echo "FAIL  $1"
    failures=$((failures + 1))
}

warn() {
    echo "WARN  $1"
    warnings=$((warnings + 1))
}

require_file() {
    if [[ -s "$1" ]]; then
        pass "$1 exists and is non-empty"
    else
        fail "$1 is missing or empty"
    fi
}

require_file client/ui/assets/ankai-night-city.png
require_file client/ui/assets/ankai-loading-idol.png
require_file client/ui/player-overlay.slint
require_file client/ui/loading-idol.slint
require_file client/src/images.rs
require_file core/src/stremio.rs

# Every callback exported by AppWindow is an API contract. A visual control
# can invoke it successfully only if Rust installs the generated on_* handler.
callback_file="$(mktemp)"
trap 'rm -f "$callback_file"' EXIT
awk '
    /^export component AppWindow inherits Window/ { in_app = 1 }
    in_app && /^    callback [a-z0-9-]+\(/ {
        name = $2
        sub(/\(.*/, "", name)
        print name
    }
' client/ui/app.slint > "$callback_file"

missing_callbacks=0
while IFS= read -r callback; do
    [[ -z "$callback" ]] && continue
    rust_callback="${callback//-/_}"
    if ! rg -q "app\\.on_${rust_callback}\\(" client/src/main.rs; then
        echo "      missing Rust handler: $callback (on_${rust_callback})"
        missing_callbacks=$((missing_callbacks + 1))
    fi
done < "$callback_file"
if [[ "$missing_callbacks" -eq 0 ]]; then
    pass "all exported AppWindow callbacks have Rust handlers"
else
    fail "$missing_callbacks exported AppWindow callback(s) have no Rust handler"
fi

# MAL discussion/forum code was explicitly retired. Jikan rating strings and
# its typed read-only client are intentionally not rejected by this check.
if rg -n -i 'mal_forums|mal forums|myanimelist forums' \
    client/src client/ui core/src/lib.rs core/src 2>/dev/null; then
    fail "retired MyAnimeList forum surface is still reachable or registered"
else
    pass "retired MyAnimeList forum surface is absent"
fi

if rg -q -i 'synopsis|episode|anime[ -]type|has-poster|metadata and ratings' \
    client/ui/anime-discovery.slint; then
    warn "Jikan's reachable UI exposes more than the requested rating-only surface"
else
    pass "Jikan's reachable UI is rating-only"
fi

# Readiness warnings below are deliberately concrete. They are non-fatal in
# normal development so the script remains useful while a phase is in flight;
# release candidates should run this script with --strict.
if rg -q 'accessible-role: button' client/ui/shell-components.slint \
    && rg -q 'accessible-action-default' client/ui/shell-components.slint; then
    pass "primary shell navigation exposes button semantics and default actions"
else
    warn "primary shell navigation is TouchArea-only; add accessible button roles, labels, and default actions"
fi

reduced_motion_files="$(rg -l 'reduced-motion' client/ui client/src 2>/dev/null || true)"
reduced_motion_count="$(printf '%s\n' "$reduced_motion_files" | sed '/^$/d' | wc -l | tr -d ' ')"
if [[ "$reduced_motion_count" -gt 1 ]]; then
    pass "loading animation reduced-motion preference is wired outside its component"
else
    warn "LoadingIdol has reduced-motion support but AppWindow/settings never supplies it"
fi
animated_ui_count="$(rg -l 'animate ' client/ui/*.slint 2>/dev/null | wc -l | tr -d ' ')"
motion_aware_ui_count="$(rg -l 'reduced-motion' client/ui/*.slint 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$motion_aware_ui_count" -ge "$animated_ui_count" ]]; then
    pass "all animated UI files declare reduced-motion behavior"
else
    warn "reduced motion reaches the loader, but other animated UI surfaces do not consume it"
fi

if rg -q 'MAX_(IMAGE|RESPONSE|DOWNLOAD).*BYTES|content_length\(\)' client/src/images.rs \
    && rg -q 'timeout\(' client/src/images.rs; then
    pass "remote image downloads have time and byte bounds"
else
    warn "remote image fetch/decode has no explicit timeout and/or byte/dimension bound"
fi

if rg -q 'timeout\(' core/src/stremio.rs \
    && rg -q 'MAX_.*(BODY|RESPONSE).*BYTES|content_length\(\)' core/src/stremio.rs; then
    pass "custom Stremio addon responses have time and byte bounds"
else
    warn "custom Stremio addon requests have no explicit timeout and/or response-size bound"
fi

if rg -q 'is_loopback|is_private|IpAddr|localhost' core/src/stremio.rs; then
    pass "custom addon URL validation rejects literal/local-name private targets"
else
    warn "custom addon URLs can target loopback/private-network services; document or gate this trust boundary"
fi
if rg -q 'lookup_host|resolve_host|dns_resolver|SocketAddr' core/src/stremio.rs; then
    pass "custom addon hostname resolution is checked/pinned against private-address results"
else
    warn "addon/image hostnames are not resolved and pinned, leaving DNS-to-private-address rebinding open"
fi

if rg -q -i 'validate_(direct_)?stream|allowed.*scheme|stream URL must use|validate_public_https_url\(&parsed, "direct stream URL"\)' \
    core/src/stremio.rs client/src/main.rs; then
    pass "direct addon stream URLs cross an explicit scheme policy before libmpv"
else
    warn "addon-provided direct stream URLs reach libmpv without an explicit scheme policy"
fi

if rg -q 'app\.set_player_buffered\(state\.position_seconds' client/src/main.rs; then
    warn "player buffered duration is copied from playhead position instead of a real buffer metric"
else
    pass "player does not present playhead position as buffered duration"
fi

if rg -q 'set_fullscreen\(false\)' client/src/main.rs; then
    pass "player close/error paths can explicitly leave fullscreen"
else
    warn "player close does not explicitly leave fullscreen"
fi

if rg -q 'struct RequestGeneration|REQUEST_GENERATION|request_generation|AbortHandle|CancellationToken' client/src/main.rs; then
    pass "async catalog/search surfaces have a stale-result cancellation or generation guard"
else
    warn "async catalog/search results have no shared last-request-wins guard"
fi

if rg -q 'width: 196px' client/ui/app.slint \
    && ! rg -q 'compact|bottom.navigation|nav-collapsed' client/ui/shell-components.slint; then
    warn "the 196px primary sidebar has no compact/mobile shell breakpoint"
else
    pass "primary navigation declares a compact shell policy"
fi
if rg -q 'width: (340|500)px' client/ui/app.slint; then
    warn "legacy Messages/Profile/Settings panes retain fixed-width controls below the compact shell"
else
    pass "legacy panes avoid known fixed-width overflow controls"
fi
if rg -q 'min-width: (560|640)px' \
    client/ui/player-overlay.slint client/ui/hangout-player-surface.slint; then
    warn "player/hangout surfaces retain minimum widths wider than the compact shell at 560px"
else
    pass "player and hangout surfaces can shrink to the compact shell's narrow-window content width"
fi

glass_button_block="$(sed -n '/^component GlassIconButton /,/^}/p' client/ui/app.slint)"
if printf '%s\n' "$glass_button_block" | rg -q 'accessible-role: button' \
    && printf '%s\n' "$glass_button_block" | rg -q 'FocusScope' \
    && rg -q 'FocusScope' client/ui/home-dashboard.slint \
    && rg -q 'FocusScope' client/ui/title-detail.slint \
    && ! rg -q 'media-touch := TouchArea|room-touch := TouchArea|^[[:space:]]+TouchArea \{' \
        client/ui/app.slint; then
    pass "reachable custom controls have keyboard and accessibility semantics"
else
    warn "reachable community/hangout/catalog rows or custom controls remain mouse-only"
fi

if rg -q 'Notifications are not implemented yet|Guestbook isn.t built yet' \
    client/src/main.rs client/ui/app.slint; then
    warn "reachable notification/guestbook affordance still ends in a placeholder"
else
    pass "reachable top-level affordances do not terminate in known placeholders"
fi

startup_expects="$(rg -c '\.expect\(' client/src/main.rs || true)"
if [[ "$startup_expects" -eq 0 ]]; then
    pass "client startup contains no fatal expect paths"
else
    warn "client startup contains $startup_expects fatal expect path(s); optional social/P2P failures should degrade in-app"
fi

if [[ "$run_full" -eq 1 ]]; then
    if cargo fmt --all --check; then
        pass "Rust formatting"
    else
        fail "Rust formatting differs from cargo fmt"
    fi

    if cargo check -p client --all-targets; then
        pass "client and Slint UI compile"
    else
        fail "client or Slint UI does not compile"
    fi

    if cargo clippy -p client --all-targets -- -D warnings; then
        pass "client strict clippy"
    else
        fail "client strict clippy"
    fi

    if cargo test -p ankai-core --lib; then
        pass "ankai-core library tests"
    else
        fail "ankai-core library tests"
    fi
fi

if [[ "$strict" -eq 1 && "$warnings" -gt 0 ]]; then
    failures=$((failures + warnings))
fi

echo "SUMMARY failures=$failures warnings=$warnings strict=$strict full=$run_full"
[[ "$failures" -eq 0 ]]
