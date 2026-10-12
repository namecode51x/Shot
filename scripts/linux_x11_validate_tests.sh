#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Non-interactive SAFETY tests for scripts/linux_x11_validate.sh (M1b-v R1).
#
# What this proves (no X server, no cargo, no network, no GUI input, and it
# NEVER runs the harness main flow):
#   * the harness refuses to run its main flow under test mode / on a
#     non-Linux OS / outside the repository;
#   * an ambient (real desktop) DISPLAY is recorded then dropped;
#   * automated GUI input is REFUSED unless this process owns a private
#     Xvfb and NSH_DISPLAY points at that very display (incl. tamper case),
#     and the refusal happens BEFORE any command string executes;
#   * build/test/X commands are bounded by timeouts;
#   * app waits are bounded and termination is graceful + scoped;
#   * cleanup only ever touches pids the harness itself recorded;
#   * the private-display picker skips occupied displays;
#   * missing T3 tools / offline deps produce the exact prerequisite report;
#   * static properties of the harness: offline, no installs, no elevated
#     privileges, no clipboard tools, no pattern-based kills, temp-dir
#     evidence, trap present, NOT TESTED markers present.
#
# Usage: bash scripts/linux_x11_validate_tests.sh
# Exit:  0 = all assertions passed, 1 = at least one failed.
# Side effects: creates tiny scratch dirs under ${TMPDIR:-/tmp}; deletes
# nothing, anywhere (evidence-presence is part of the tested contract).
# ---------------------------------------------------------------------------
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
SCRIPT="$SCRIPT_DIR/linux_x11_validate.sh"

if [ ! -f "$SCRIPT" ]; then
    echo "FATAL: $SCRIPT not found" >&2
    exit 2
fi

# Sourcing defines functions + defaults only; main() is guarded and never
# runs here (BASH_SOURCE != $0).
# shellcheck source=linux_x11_validate.sh
. "$SCRIPT"

T=0; F=0
t_ok()   { T=$((T + 1)); printf 'ok %d - %s\n' "$T" "$1"; }
t_fail() { T=$((T + 1)); F=$((F + 1)); printf 'not ok %d - %s%s\n' "$T" "$1" "${2:+ ($2)}"; }
assert_eq()    { if [ "$2" = "$3" ]; then t_ok "$1"; else t_fail "$1" "got [$2] want [$3]"; fi; }
assert_in()    { case "$2" in *"$3"*) t_ok "$1" ;; *) t_fail "$1" "missing [$3]" ;; esac; }
assert_not_in() { case "$2" in *"$3"*) t_fail "$1" "unexpected [$3]" ;; *) t_ok "$1" ;; esac; }
assert_le()    { if [ "$2" -le "$3" ]; then t_ok "$1"; else t_fail "$1" "got $2 > max $3"; fi; }
assert_lt()    { if [ "$2" -lt "$3" ]; then t_ok "$1"; else t_fail "$1" "got $2 >= $3"; fi; }

printf '# harness safety tests: %s\n' "$SCRIPT"

# ---------------------------------------------------------------------------
printf '# static safety properties of the harness\n'
TXT=$(cat "$SCRIPT")
for needle in sudo 'apt-get' 'apt install' curl wget 'yum ' 'dnf ' pacman \
              xclip xsel wl-copy wl-paste pkill killall 'rm -'; do
    assert_not_in "harness contains no '$needle'" "$TXT" "$needle"
done
assert_in "harness forces cargo offline (CARGO_NET_OFFLINE=true)" "$TXT" "CARGO_NET_OFFLINE=true"
assert_in "harness passes --offline to cargo" "$TXT" "--offline"
assert_in "harness bounds commands with timeout" "$TXT" "timeout -k"
assert_in "harness writes evidence to a unique mktemp dir" "$TXT" "mktemp -d"
assert_in "harness evidence lives under \$NSH_OUT (not the repo root)" "$TXT" '$NSH_OUT/validation-results-'
assert_in "harness registers scoped cleanup trap" "$TXT" "trap 'nsh_cleanup' EXIT"
assert_in "harness starts Xvfb without TCP listeners" "$TXT" "-nolisten tcp"
assert_in "harness drops ambient DISPLAY" "$TXT" "unset DISPLAY"
assert_in "clipboard check marked NOT TESTED" "$TXT" "CLIPBOARD: NOT TESTED"
assert_in "hotkey check marked NOT TESTED" "$TXT" "GLOBAL HOTKEYS: NOT TESTED"
NT_COUNT=$(printf '%s\n' "$TXT" | grep -c 'NOT TESTED' || true)
if [ "$NT_COUNT" -ge 2 ]; then t_ok "at least two NOT TESTED markers"; else t_fail "at least two NOT TESTED markers" "found $NT_COUNT"; fi

# ---------------------------------------------------------------------------
printf '# guard refusals (never launching the real harness)\n'
if ! command -v timeout >/dev/null 2>&1; then
    t_fail "coreutils 'timeout' available (required for the harness and these tests)"
else
    t_ok "coreutils 'timeout' available"
fi

OUT=$(NSH_TEST_MODE=1 nsh_guard 2>&1); RC=$?
assert_eq "test mode refuses main flow (exit 2)" "$RC" "2"
assert_in "test mode refusal is explicit" "$OUT" "NSH_TEST_MODE"

TDIR=$(mktemp -d "${TMPDIR:-/tmp}/nameshot-vtest-guard.XXXXXX")
cp "$SCRIPT" "$TDIR/linux_x11_validate.sh"
OUT=$(
    unset NSH_TEST_MODE
    . "$TDIR/linux_x11_validate.sh"
    nsh_guard 2>&1
); RC=$?
assert_eq "non-repository location refused (exit 2)" "$RC" "2"
assert_in "non-repository refusal is explicit" "$OUT" "NameShot root"

case "$(uname -s)" in
    Linux)
        OUT=$(unset NSH_TEST_MODE; nsh_guard 2>&1); RC=$?
        assert_eq "Linux host passes the OS guard" "$RC" "0"
        ;;
    *)
        OUT=$(unset NSH_TEST_MODE; nsh_guard 2>&1); RC=$?
        assert_eq "non-Linux host refused by OS guard (exit 2)" "$RC" "2"
        assert_in "OS guard refusal names the OS" "$OUT" "refusing to run on"
        ;;
esac

# ---------------------------------------------------------------------------
printf '# ambient DISPLAY is recorded then dropped (never a fallback)\n'
OUT=$(DISPLAY=:0 nsh_drop_ambient_display; printf 'D=%s|A=%s' "${DISPLAY:-EMPTY}" "$NSH_AMBIENT_DISPLAY")
assert_eq "ambient :0 recorded and DISPLAY emptied" "$OUT" "D=EMPTY|A=:0"

# ---------------------------------------------------------------------------
printf '# isolation guard: GUI input only on our private Xvfb\n'
OUT=$(
    unset NSH_OWNED_XVFB NSH_OWNED_DISPLAY
    NSH_DISPLAY=""
    DISPLAY=:0
    nsh_gui_input_allowed
    echo $?
)
assert_eq "real/ambient display refused (rc 1)" "$OUT" "1"

OUT=$(
    NSH_OWNED_XVFB=1
    NSH_OWNED_DISPLAY=:97
    NSH_DISPLAY=:0
    nsh_gui_input_allowed
    echo $?
)
assert_eq "tampered NSH_DISPLAY vs owned display refused (rc 1)" "$OUT" "1"

OUT=$(
    NSH_OWNED_XVFB=1
    NSH_OWNED_DISPLAY=:97
    NSH_DISPLAY=:97
    nsh_gui_input_allowed
    echo $?
)
assert_eq "owned private display allowed (rc 0)" "$OUT" "0"

OUT=$(
    unset NSH_OWNED_XVFB NSH_OWNED_DISPLAY NSH_DISPLAY
    DISPLAY=:0
    run_x T3 "echo SHOULD_NOT_RUN"
    echo "rc=$?"
)
assert_not_in "run_x on ambient display executes nothing" "$OUT" "SHOULD_NOT_RUN"
assert_in "run_x on ambient display reports the refusal" "$OUT" "REFUSED"
assert_in "run_x returns refusal code 99" "$OUT" "rc=99"

OUT=$(
    NSH_OWNED_XVFB=1
    NSH_OWNED_DISPLAY=:97
    NSH_DISPLAY=:97
    run_x T3 "echo ISOLATED_OK"
)
assert_in "run_x executes on the owned private display" "$OUT" "ISOLATED_OK"

# ---------------------------------------------------------------------------
printf '# timeout handling\n'
T0=$SECONDS
OUT=$(nsh_timed T1 1 sleep 30 2>&1); RC=$?
DT=$((SECONDS - T0))
assert_eq "command timeout returns 124" "$RC" "124"
assert_le "command timeout is bounded (<=6s wall)" "$DT" 6
assert_in "timeout is recorded in the log line" "$OUT" "TIMED OUT"

OUT=$(
    NSH_TIMEOUT_TERM_GRACE=1
    sleep 30 & p=$!
    T0=$SECONDS
    nsh_wait_bounded "$p" 2
    RC=$?
    DT=$((SECONDS - T0))
    ST=$(kill -0 "$p" 2>/dev/null && echo running || echo dead)
    nsh_term_own "$p"
    ST2=$(kill -0 "$p" 2>/dev/null && echo running || echo dead)
    echo "rc=$RC dt=$DT st=$ST st2=$ST2"
)
assert_in "app wait timeout returns 1" "$OUT" "rc=1"
assert_in "app still running at the bound" "$OUT" "st=running"
assert_in "graceful termination then kills our own pid" "$OUT" "st2=dead"
DT=$(printf '%s' "$OUT" | sed -n 's/.*dt=\([0-9]*\).*/\1/p')
assert_le "app wait timeout is bounded (<=6s wall)" "${DT:-99}" 6

OUT=$(
    sleep 1 & p=$!
    nsh_wait_bounded "$p" 10
    echo "rc=$? waitrc=$NSH_WAIT_RC"
)
assert_in "app wait succeeds on early exit" "$OUT" "rc=0 waitrc=0"

# ---------------------------------------------------------------------------
printf '# cleanup scope: only pids recorded by the harness\n'
OUT=$(
    NSH_TIMEOUT_TERM_GRACE=1
    NSH_CLEANED=0
    NSH_APP_PIDS=""
    NSH_XVFB_PID=""
    NSH_OUT=""
    LOG=""
    sleep 60 & own=$!
    sleep 60 & foreign=$!
    NSH_APP_PIDS="$own"
    nsh_cleanup
    O=$(kill -0 "$own" 2>/dev/null && echo alive || echo dead)
    X=$(kill -0 "$foreign" 2>/dev/null && echo alive || echo dead)
    printf 'own=%s foreign=%s' "$O" "$X"
    kill -TERM "$foreign" 2>/dev/null
    wait "$foreign" 2>/dev/null
)
assert_in "script-owned pid is terminated by cleanup" "$OUT" "own=dead"
assert_in "unrelated (foreign) pid is never touched" "$OUT" "foreign=alive"

# ---------------------------------------------------------------------------
printf '# private display picker skips occupied displays\n'
TD1=$(mktemp -d "${TMPDIR:-/tmp}/nameshot-vtest-dpy.XXXXXX")
mkdir -p "$TD1/.X11-unix"
: >"$TD1/.X11-unix/X90"
: >"$TD1/.X91-lock"
GOT=$(NSH_X_TMP="$TD1" nsh_free_display); RC=$?
assert_eq "occupied socket+lock => next free display" "$GOT" ":92"
assert_eq "picker succeeds (exit 0)" "$RC" "0"
TD2=$(mktemp -d "${TMPDIR:-/tmp}/nameshot-vtest-dpy.XXXXXX")
mkdir -p "$TD2/.X11-unix"
GOT=$(NSH_X_TMP="$TD2" nsh_free_display); RC=$?
assert_eq "empty dir => first display" "$GOT" ":90"
assert_eq "picker succeeds (exit 0)" "$RC" "0"

# ---------------------------------------------------------------------------
printf '# prerequisite reports (missing tools / missing offline cache)\n'
EB=$(mktemp -d "${TMPDIR:-/tmp}/nameshot-vtest-path.XXXXXX")
OUT=$(PATH="$EB" nsh_t3_prereqs 2>&1); RC=$?
assert_eq "missing T3 toolset => nonzero" "$RC" "1"
assert_in "missing T3 toolset names Xvfb" "$OUT" "Xvfb"
assert_in "missing T3 toolset lists the exact package" "$OUT" "xvfb"
assert_in "missing T3 toolset refuses to fall back" "$OUT" "NOT fall back"

OUT=$(PATH="$EB" nsh_offline_preflight 2>&1); RC=$?
assert_eq "offline cache failure => nonzero" "$RC" "1"
assert_in "offline failure reports a PREREQUISITE" "$OUT" "PREREQUISITE"
assert_in "offline failure gives the exact remedy" "$OUT" "cargo fetch"

# ---------------------------------------------------------------------------
printf '# summary\n'
printf '1..%d\n' "$T"
if [ "$F" -eq 0 ]; then
    printf 'PASS: %d/%d assertions\n' "$T" "$T"
    exit 0
fi
printf 'FAIL: %d of %d assertions failed\n' "$F" "$T"
exit 1
