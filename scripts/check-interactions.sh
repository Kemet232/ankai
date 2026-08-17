#!/usr/bin/env bash
# Static interaction-contract checks for ANKAI's reachable desktop UI.
#
# Default mode fails only on broken contracts. `--strict` promotes actionable
# warnings to failures and is intended for the release-candidate CI job.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root" || exit 1

strict=0
if [[ "${1:-}" == "--strict" ]]; then
    strict=1
elif [[ "$#" -ne 0 ]]; then
    echo "usage: $0 [--strict]" >&2
    exit 2
fi

failures=0
warnings=0

pass() {
    echo "PASS  $1"
}

warn() {
    echo "WARN  $1"
    warnings=$((warnings + 1))
}

fail() {
    echo "FAIL  $1"
    failures=$((failures + 1))
}

for command in rg awk sed sort uniq mktemp; do
    if ! command -v "$command" >/dev/null 2>&1; then
        fail "required command is unavailable: $command"
    fi
done
if [[ "$failures" -ne 0 ]]; then
    echo "SUMMARY failures=$failures warnings=$warnings strict=$strict"
    exit 1
fi

app_ui="client/ui/app.slint"
rust_host="client/src/main.rs"
production_ui=(
    client/ui/app.slint
    client/ui/addon-manager.slint
    client/ui/anime-discovery.slint
    client/ui/home-dashboard.slint
    client/ui/hangout-player-surface.slint
    client/ui/letterboxd-feed.slint
    client/ui/nyaa-releases.slint
    client/ui/player-overlay.slint
    client/ui/shell-components.slint
    client/ui/title-detail.slint
)

for file in "$app_ui" "$rust_host" "${production_ui[@]}"; do
    if [[ ! -s "$file" ]]; then
        fail "required interaction source is missing or empty: $file"
    fi
done

callbacks_file="$(mktemp)"
routes_file="$(mktemp)"
route_indexes_file="$(mktemp)"
touch_gaps_file="$(mktemp)"
trap 'rm -f "$callbacks_file" "$routes_file" "$route_indexes_file" "$touch_gaps_file"' EXIT

# AppWindow callbacks are the generated Rust API. Missing an on_* registration
# is a deterministic dead-control defect, so this is a hard failure.
awk '
    /^export component AppWindow inherits Window/ { in_app = 1 }
    in_app && /^    callback [a-z0-9-]+\(/ {
        name = $2
        sub(/\(.*/, "", name)
        print name
    }
' "$app_ui" | sort -u > "$callbacks_file"

callback_count="$(wc -l < "$callbacks_file" | tr -d ' ')"
missing_callbacks=0
while IFS= read -r callback; do
    [[ -z "$callback" ]] && continue
    rust_callback="${callback//-/_}"
    if ! rg -q "app\\.on_${rust_callback}\\(" "$rust_host"; then
        echo "      missing Rust handler: $callback (app.on_${rust_callback})"
        missing_callbacks=$((missing_callbacks + 1))
    fi
done < "$callbacks_file"
if [[ "$callback_count" -eq 0 ]]; then
    fail "AppWindow exports no callbacks; parser or UI contract changed"
elif [[ "$missing_callbacks" -eq 0 ]]; then
    pass "all $callback_count exported AppWindow callbacks have Rust handlers"
else
    fail "$missing_callbacks exported AppWindow callback(s) have no Rust handler"
fi

# Parse nav labels and the named route-index properties. Every named route must
# have one in-range, unique index and at least one reachable pane condition.
awk '
    /property <\[string\]> nav-items:/ { in_nav = 1; next }
    in_nav && /\];/ { in_nav = 0; exit }
    in_nav {
        line = $0
        while (match(line, /"[^"]+"/)) {
            label = substr(line, RSTART + 1, RLENGTH - 2)
            print label
            line = substr(line, RSTART + RLENGTH)
        }
    }
' "$app_ui" > "$routes_file"

sed -n 's/^[[:space:]]*out property <int> \([a-z0-9-]*-index\): \([0-9][0-9]*\);.*/\1 \2/p' \
    "$app_ui" > "$route_indexes_file"

route_count="$(wc -l < "$routes_file" | tr -d ' ')"
route_index_count="$(wc -l < "$route_indexes_file" | tr -d ' ')"
duplicate_route_labels="$(sort "$routes_file" | uniq -d)"
duplicate_route_indexes="$(awk '{ print $2 }' "$route_indexes_file" | sort | uniq -d)"
route_errors=0
if [[ "$route_count" -eq 0 ]]; then
    echo "      nav-items contains no named routes"
    route_errors=$((route_errors + 1))
fi
if [[ "$route_count" -ne "$route_index_count" ]]; then
    echo "      route labels ($route_count) and named indexes ($route_index_count) differ"
    route_errors=$((route_errors + 1))
fi
if [[ -n "$duplicate_route_labels" ]]; then
    echo "      duplicate route label(s): $duplicate_route_labels"
    route_errors=$((route_errors + 1))
fi
if [[ -n "$duplicate_route_indexes" ]]; then
    echo "      duplicate route index(es): $duplicate_route_indexes"
    route_errors=$((route_errors + 1))
fi
while read -r route_property route_index; do
    [[ -z "${route_property:-}" ]] && continue
    if (( route_index < 0 || route_index >= route_count )); then
        echo "      out-of-range route: $route_property=$route_index (count=$route_count)"
        route_errors=$((route_errors + 1))
    fi
    if ! rg -q "selected-index == root\\.${route_property}" "$app_ui"; then
        echo "      route has no reachable pane condition: $route_property"
        route_errors=$((route_errors + 1))
    fi
done < "$route_indexes_file"
if [[ "$route_errors" -eq 0 ]]; then
    pass "$route_count named top-level routes have unique, in-range, reachable indexes"
else
    fail "$route_errors top-level route contract error(s)"
fi

# Extract actionable TouchArea blocks. Every clicked/dragged production control
# must hand focus to a FocusScope. Hover-only card TouchAreas are intentionally
# ignored. The compiler validates that the named focus target actually exists.
awk '
    function braces(text, copy, opens, closes) {
        copy = text
        opens = gsub(/\{/, "", copy)
        copy = text
        closes = gsub(/\}/, "", copy)
        return opens - closes
    }
    function finish_block() {
        actionable = block ~ /(clicked|double-clicked)[[:space:]]*=>/ || block ~ /changed[[:space:]]+pressed[[:space:]]*=>/
        if (actionable && block !~ /[A-Za-z0-9_-]+\.focus\(\)/) {
            print FILENAME ":" start_line ": actionable TouchArea does not hand focus to a keyboard scope"
        }
        inside = 0
        block = ""
        depth = 0
    }
    {
        if (!inside && $0 ~ /TouchArea[[:space:]]*\{/) {
            inside = 1
            start_line = FNR
            block = $0 "\n"
            depth = braces($0)
            if (depth <= 0) { finish_block() }
            next
        }
        if (inside) {
            block = block $0 "\n"
            depth += braces($0)
            if (depth <= 0) { finish_block() }
        }
    }
    END {
        if (inside) { print FILENAME ":" start_line ": unterminated TouchArea block" }
    }
' "${production_ui[@]}" > "$touch_gaps_file"

if [[ -s "$touch_gaps_file" ]]; then
    sed 's/^/      /' "$touch_gaps_file"
    touch_gap_count="$(wc -l < "$touch_gaps_file" | tr -d ' ')"
    warn "$touch_gap_count actionable TouchArea control(s) have no keyboard-focus handoff"
else
    pass "all actionable production TouchAreas hand focus to keyboard scopes"
fi

semantic_file_gaps=0
for file in "${production_ui[@]}"; do
    if rg -q '(clicked|double-clicked)[[:space:]]*=>' "$file"; then
        if ! rg -q 'FocusScope' "$file" || ! rg -q 'accessible-action-(default|increment|decrement|set-value)' "$file"; then
            echo "      incomplete keyboard/a11y semantics in $file"
            semantic_file_gaps=$((semantic_file_gaps + 1))
        fi
    fi
done
if [[ "$semantic_file_gaps" -eq 0 ]]; then
    pass "production files with custom actions declare FocusScope and accessible actions"
else
    warn "$semantic_file_gaps production UI file(s) have incomplete custom-control semantics"
fi

# Fail only on known user-visible placeholder destinations. Honest disabled or
# empty/error copy (for example local-only Hangout chat) is not a placeholder.
placeholder_pattern='(text:|shell-notice[[:space:]]*=|set_shell_notice\()[^;]*(not implemented|isn.t built|coming soon|placeholder action|todo action)'
placeholder_hits="$(rg -n -i "$placeholder_pattern" "$app_ui" "$rust_host" "${production_ui[@]}" 2>/dev/null || true)"
if [[ -n "$placeholder_hits" ]]; then
    printf '%s\n' "$placeholder_hits" | sed 's/^/      /'
    fail "visible actions still point to known placeholder destinations"
else
    pass "visible action copy contains no known placeholder destination"
fi
if rg -q 'chat-enabled: false' "$app_ui" \
    && rg -q 'enabled: root\.chat-enabled' client/ui/hangout-player-surface.slint; then
    pass "local-only Hangout chat is truthfully disabled rather than presented as working"
else
    warn "local-only Hangout chat enablement/copy should be manually checked"
fi

check_state_surface() {
    label="$1"
    file="$2"
    shift 2
    missing=0
    for pattern in "$@"; do
        if ! rg -q "$pattern" "$file"; then
            missing=$((missing + 1))
        fi
    done
    if [[ "$missing" -eq 0 ]]; then
        pass "$label declares loading/error/empty/retry coverage"
    else
        warn "$label is missing $missing required async-state marker(s)"
    fi
}

check_state_surface "anime discovery" client/ui/anime-discovery.slint \
    'root\.state == "loading"|root\.state != "ready"' \
    'root\.state == "error"' 'root\.results\.length == 0' 'callback retry\(\)'
check_state_surface "addon manager" client/ui/addon-manager.slint \
    'state: "loading"|root\.state != "ready"' \
    'root\.state == "error"' 'root\.addons\.length == 0' 'callback retry\(\)'
check_state_surface "Letterboxd feed" client/ui/letterboxd-feed.slint \
    'root\.state == "loading"' 'root\.state == "error"' \
    'root\.entries\.length == 0' 'callback retry\(\)'
check_state_surface "Nyaa releases" client/ui/nyaa-releases.slint \
    'root\.state == "loading"' 'root\.state == "error"' \
    'root\.results\.length == 0' 'callback retry\(\)'
check_state_surface "embedded player" client/ui/player-overlay.slint \
    'root\.is-loading' 'root\.error-message != ""' \
    '!root\.has-media|root\.has-media' 'callback retry-requested\(\)'

if rg -q 'friends-state: "ready"' "$app_ui" \
    && rg -q 'discussions-state: "ready"' "$app_ui" \
    && rg -q 'hangouts-state: "ready"' "$app_ui"; then
    warn "Home friends/discussions/hangouts error and retry branches are unreachable because AppWindow hardcodes ready"
else
    pass "Home secondary modules receive non-constant async state"
fi
if rg -q 'trending-anime\.length == 0 \? "loading"' "$app_ui"; then
    warn "Home treats a successful empty anime result as perpetual loading"
else
    pass "Home distinguishes an empty anime result from loading"
fi

# Player and global-search interaction contract documented in
# docs/qa/interaction-audit.md.
shortcut_gaps=0
for pattern in \
    'Key\.Space' \
    'Key\.LeftArrow' \
    'Key\.RightArrow' \
    'Key\.UpArrow' \
    'Key\.DownArrow' \
    'event\.text == "m"' \
    'event\.text == "f"' \
    'Key\.Escape' \
    'root\.toggle-playback\(\)' \
    'root\.seek-relative\(' \
    'root\.volume-requested\(' \
    'root\.mute-toggled\(\)' \
    'root\.fullscreen-toggled\(\)' \
    'root\.close-requested\(\)'; do
    if ! rg -q "$pattern" client/ui/player-overlay.slint; then
        echo "      missing player shortcut/action marker: $pattern"
        shortcut_gaps=$((shortcut_gaps + 1))
    fi
done
if [[ "$shortcut_gaps" -eq 0 ]]; then
    pass "documented player keyboard shortcuts dispatch their actions"
else
    fail "$shortcut_gaps documented player shortcut marker(s) are missing"
fi

if rg -q 'accepted => \{ root\.submitted\(root\.query\); \}' client/ui/shell-components.slint \
    && rg -q 'submitted\(query\) => \{ root\.global-search\(query\); \}' "$app_ui" \
    && rg -q 'app\.on_global_search\(' "$rust_host"; then
    pass "Enter in global search reaches the Rust global-search handler"
else
    fail "global search submission is not wired end to end"
fi
if ! rg -q 'accessible-label:' client/ui/shell-components.slint \
    || ! sed -n '/LineEdit {/,/}/p' client/ui/shell-components.slint | rg -q 'accessible-label:'; then
    warn "global search LineEdit relies on placeholder text and has no explicit accessible label"
else
    pass "global search input has an explicit accessible label"
fi

# Breakpoints and minimums are policy, not just incidental layouts.
if rg -q 'compact-shell: root\.width < [0-9]+px' "$app_ui" \
    && rg -q 'compact-layout: root\.width < [0-9]+px' client/ui/home-dashboard.slint \
    && rg -q 'root\.width >= [0-9]+px' client/ui/player-overlay.slint \
    && rg -q 'root\.width >= [0-9]+px' client/ui/hangout-player-surface.slint \
    && rg -q 'root\.width < [0-9]+px' client/ui/nyaa-releases.slint; then
    pass "shell, Home, player, Hangout, and Nyaa declare compact breakpoints"
else
    warn "one or more major surfaces have no explicit compact breakpoint"
fi
if rg -q 'min-width: 320px' client/ui/player-overlay.slint \
    && rg -q 'min-width: 320px' client/ui/hangout-player-surface.slint; then
    pass "player and Hangout surfaces declare a 320px minimum width"
else
    warn "player/Hangout minimum widths are missing or no longer aligned"
fi
if rg -q '^[[:space:]]*min-width:' "$app_ui" \
    && rg -q '^[[:space:]]*min-height:' "$app_ui"; then
    pass "AppWindow declares a minimum usable size"
else
    warn "AppWindow has preferred dimensions but no explicit minimum usable size"
fi

if rg -q 'open-friend\(account-id\).*' "$app_ui" \
    && rg -q 'Opened your friends area for' "$app_ui" \
    && rg -q 'Browse .* in Communities' "$app_ui"; then
    warn "Home friend/discussion actions route broadly and do not select the requested entity"
else
    pass "Home entity actions preserve their exact destination"
fi

if [[ "$strict" -eq 1 && "$warnings" -gt 0 ]]; then
    failures=$((failures + warnings))
fi

echo "SUMMARY failures=$failures warnings=$warnings strict=$strict"
[[ "$failures" -eq 0 ]]
