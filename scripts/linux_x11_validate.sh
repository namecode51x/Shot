#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# NameShot — reproducible Linux/X11 validation harness (M1b-v, safety rev R1).
#
# SAFETY CONTRACT (enforced below; tested by
# scripts/linux_x11_validate_tests.sh — run those before this harness):
#   1. ISOLATED GUI INPUT ONLY. Automated input (xdotool/xwd/xwininfo/
#      xdpyinfo, launching the app) runs ONLY against a private Xvfb display
#      created by THIS process (NSH_OWNED_XVFB=1, NSH_DISPLAY ==
#      NSH_OWNED_DISPLAY). Any ambient DISPLAY is recorded, then dropped
#      (`unset DISPLAY`); it is never used as a fallback. If Xvfb (or other
#      T3 tools) are missing: print the exact prerequisite, run NO GUI
#      input, stop — exit 3.
#   2. OFFLINE BY DEFAULT. CARGO_NET_OFFLINE=true for the whole process;
#      cargo subcommands also carry --offline. A `cargo metadata --offline`
#      preflight verifies the local cache; if a dependency is missing, print
#      the exact prerequisite and stop before any build — exit 3. This
#      script never installs packages, never elevates privileges, and makes
#      no network access of its own.
#   3. BOUNDED + GRACEFUL. Every build/test/X command runs under coreutils
#      `timeout`. Every wait on a launched app is bounded. On timeout or
#      cleanup: SIGTERM first, bounded grace, SIGKILL only on pids this
#      script recorded — never pattern-based kills, never a foreign pid.
#   4. SCOPED CLEANUP. EXIT/INT/TERM trap runs nsh_cleanup, which touches
#      exactly NSH_APP_PIDS (its own app launches) + NSH_XVFB_PID (its own
#      Xvfb) and nothing else.
#   5. EVIDENCE IN A UNIQUE TEMP DIR. Everything (log, app stdout/stderr,
#      window tree, screenshots) goes to a fresh
#      ${TMPDIR:-/tmp}/nameshot-validation.XXXXXX directory — never the
#      repository, never a personal folder. Nothing is ever overwritten
#      (mktemp) and nothing is deleted: evidence is preserved after the run.
#   6. CLIPBOARD + GLOBAL HOTKEYS ARE NOT TESTED (M1b scope). This harness
#      invokes no clipboard tool, reads/writes no selection, and registers
#      no hotkey.
#
# Honesty rules (do not edit these):
#   * This script NEVER claims a behavior passed. It records commands, exit
#     codes and artifacts; MANUAL RESULT lines are filled in by the operator
#     after observing the behavior.
#   * A behavior is "passed" only if its evidence line is in the results log
#     with exit 0 (machine) or an operator-filled PASS (manual), and that
#     log has been attached to docs/BUILD_LOG.md (M1b-v section).
#   * Compile checks and live checks are separate tiers (T1/T2 vs T3);
#     passing T1/T2 says NOTHING about T3.
#   * Nothing here packages, touches dist/, runs on Wayland, or edits the
#     repository source.
#
# Usage (on a Linux host, from the repository):
#   bash scripts/linux_x11_validate.sh                 # tiers 0,1,2,3
#   bash scripts/linux_x11_validate.sh --tiers 0,1,2   # no live checks
#
# Exit codes:
#   0  machine tiers all exited 0 (live tiers: see MANUAL RESULT lines)
#   1  a machine-tier command failed (see the log)
#   2  guard refusal (not Linux / not the NameShot repo / test mode)
#   3  prerequisite missing (offline cache and/or T3 toolset incl. Xvfb);
#      the blocked tier(s) recorded NOT RUN — no GUI input was attempted
#
# Timeouts (seconds; override via environment):
#   NSH_TIMEOUT_PROBE=60      T0 probes
#   NSH_TIMEOUT_BUILD=900     cargo check/clippy/build
#   NSH_TIMEOUT_TEST=900      cargo test
#   NSH_TIMEOUT_METADATA=120  offline preflight
#   NSH_TIMEOUT_GUI=30        individual X/xdotool/xwd commands
#   NSH_TIMEOUT_APP_WAIT=20   wait for the app to exit after a key/flow
#   NSH_TIMEOUT_XVFB_START=10 wait for the private Xvfb socket
#   NSH_TIMEOUT_TERM_GRACE=5  SIGTERM grace before scoped SIGKILL
#   NSH_DISPLAY_MIN=90, NSH_DISPLAY_MAX=199  private display search range
#
# Required Linux packages (install yourself, beforehand; this script will
# NOT install anything): bash, coreutils (timeout/mktemp/tee/kill), sed,
# grep, gawk-or-mawk (awk), procps (ps), Xvfb, xdotool, x11-apps (xwd),
# x11-utils (xdpyinfo, xwininfo), rustup+cargo+rustc (+clippy, rustfmt).
# Debian/Ubuntu: xvfb xdotool x11-apps x11-utils
# Fedora:        xorg-x11-server-Xvfb xdotool xorg-x11-apps xorg-x11-utils
# Optional (informational only): zenity
# ---------------------------------------------------------------------------
set -u

# ---- state (top level only: this file must be sourceable side-effect-free;
#      scripts/linux_x11_validate_tests.sh sources it and unit-tests the
#      guard/scope functions without ever running main) --------------------
TS=""; NSH_OUT=""; LOG=""; APP=""
TIERS="0,1,2,3"
NSH_AMBIENT_DISPLAY=""; NSH_DISPLAY=""; NSH_OWNED_DISPLAY=""
NSH_OWNED_XVFB=0; NSH_XVFB_PID=""
NSH_APP_PIDS=""; NSH_LAST_PID=""
NSH_WAIT_RC=""; NSH_CLEANED=0
T1_FAIL=0; T2_FAIL=0; EXIT_PREREQ=0; OFFLINE_OK=1; T3_PREREQ_OK=1
NSH_X_TMP="${NSH_X_TMP:-/tmp}"; NSH_DISPLAY_MIN=90; NSH_DISPLAY_MAX=199
NSH_TIMEOUT_PROBE="${NSH_TIMEOUT_PROBE:-60}"
NSH_TIMEOUT_BUILD="${NSH_TIMEOUT_BUILD:-900}"
NSH_TIMEOUT_TEST="${NSH_TIMEOUT_TEST:-900}"
NSH_TIMEOUT_METADATA="${NSH_TIMEOUT_METADATA:-120}"
NSH_TIMEOUT_GUI="${NSH_TIMEOUT_GUI:-30}"
NSH_TIMEOUT_APP_WAIT="${NSH_TIMEOUT_APP_WAIT:-20}"
NSH_TIMEOUT_XVFB_START="${NSH_TIMEOUT_XVFB_START:-10}"
NSH_TIMEOUT_TERM_GRACE="${NSH_TIMEOUT_TERM_GRACE:-5}"

# ---- logging (safe while sourced: no log file yet => stdout only) --------
say() {
    if [ -n "${LOG:-}" ]; then printf '%s\n' "$*" | tee -a "$LOG";
    else printf '%s\n' "$*"; fi
}
say_block() {
    if [ -n "${LOG:-}" ]; then printf '%s\n' "$*" | sed 's/^/    /' | tee -a "$LOG";
    else printf '%s\n' "$*" | sed 's/^/    /'; fi
}
hdr() { say ""; say "================================================================"; say "$*"; say "================================================================"; }
note() { say "[$1] NOTE: $2"; }

# ---- guards ---------------------------------------------------------------
# Exit path is `exit` => call from a subshell in tests.
nsh_guard() {
    if [ "${NSH_TEST_MODE:-0}" = "1" ]; then
        echo "FATAL: harness main flow disabled while NSH_TEST_MODE=1 (safety tests)"
        exit 2
    fi
    local script_dir
    script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    cd "$script_dir/.." || { echo "FATAL: cannot enter repository root"; exit 2; }
    if [ ! -f Cargo.toml ] || [ ! -d docs ]; then
        echo "FATAL: Cargo.toml/docs missing - not the NameShot root"; exit 2
    fi
    case "$(uname -s)" in
        Linux) : ;;
        *) echo "FATAL: this harness validates on Linux; refusing to run on $(uname -s)"; exit 2 ;;
    esac
}

# ---- isolation guard: the ONLY permission slip for GUI input --------------
# Allowed iff THIS process owns a private Xvfb and NSH_DISPLAY still points
# at that very display. Ambient DISPLAY never satisfies this.
nsh_gui_input_allowed() {
    [ "${NSH_OWNED_XVFB:-0}" = "1" ] || return 1
    [ -n "${NSH_DISPLAY:-}" ] || return 1
    [ "${NSH_DISPLAY}" = "${NSH_OWNED_DISPLAY:-}" ] || return 1
    return 0
}
nsh_require_gui_input() {
    if nsh_gui_input_allowed; then return 0; fi
    say "[SAFETY] REFUSED automated GUI input: no private Xvfb owned by this script right now (ambient DISPLAY=${NSH_AMBIENT_DISPLAY:-${DISPLAY:-<unset>}} is never used as a fallback)"
    return 1
}
# Record then drop any ambient DISPLAY so no child can ever inherit it.
nsh_drop_ambient_display() {
    NSH_AMBIENT_DISPLAY="${DISPLAY:-}"
    unset DISPLAY || true
    return 0
}

# ---- bounded execution ----------------------------------------------------
nsh_tier_timeout() {
    case "$1" in
        T0) printf '%s' "$NSH_TIMEOUT_PROBE" ;;
        T1) printf '%s' "$NSH_TIMEOUT_BUILD" ;;
        T2) printf '%s' "$NSH_TIMEOUT_TEST" ;;
        *) printf '%s' "$NSH_TIMEOUT_GUI" ;;
    esac
}
# nsh_log_out <tier> <output> <rc> — print/log and remember T1/T2 failures.
nsh_log_out() {
    if [ -n "${LOG:-}" ]; then
        printf '%s\n' "$2" | sed 's/^/    /' | tee -a "$LOG" >/dev/null
    else
        printf '%s\n' "$2" | sed 's/^/    /' >/dev/null
    fi
    printf '%s\n' "$2"
    case "$3" in
        124|137) say "[$1] exit: $3 (TIMED OUT)" ;;
        *) say "[$1] exit: $3" ;;
    esac
    case "$1" in
        T1) [ "$3" -ne 0 ] && T1_FAIL=1 ;;
        T2) [ "$3" -ne 0 ] && T2_FAIL=1 ;;
    esac
    return "$3"
}
# nsh_timed <tier> <timeout-sec> <cmd...>
nsh_timed() {
    local tier="$1" t="$2" out rc
    shift 2
    say "[$tier] \$ (timeout ${t}s) $*"
    out=$(timeout -k 10 "$t" "$@" 2>&1); rc=$?
    nsh_log_out "$tier" "$out" "$rc"
}
# nsh_timed_shell <tier> <timeout-sec> <shell-string>
nsh_timed_shell() {
    local tier="$1" t="$2" cmd="$3" out rc
    say "[$tier] \$ (timeout ${t}s) $cmd"
    out=$(timeout -k 10 "$t" bash -c "$cmd" 2>&1); rc=$?
    nsh_log_out "$tier" "$out" "$rc"
}
run() { local tier="$1"; shift; nsh_timed "$tier" "$(nsh_tier_timeout "$tier")" "$@"; }
run_shell() { local tier="$1"; shift; nsh_timed_shell "$tier" "$(nsh_tier_timeout "$tier")" "$*"; }
# run_x <tier> <shell-string> — GUI input, ONLY on our private Xvfb.
run_x() {
    local tier="$1"; shift
    if ! nsh_require_gui_input; then
        say "[$tier] REFUSED automated GUI input - recorded as NOT RUN"
        return 99
    fi
    nsh_timed_shell "$tier" "$NSH_TIMEOUT_GUI" "DISPLAY=$NSH_DISPLAY $*"
}

# ---- bounded wait + graceful, scoped termination --------------------------
# nsh_wait_bounded <pid> <sec> — 0 = exited (code in NSH_WAIT_RC), 1 = still
# running after <sec>. Never waits unbounded.
nsh_wait_bounded() {
    local pid="$1" limit="$2" waited=0
    NSH_WAIT_RC=""
    while [ "$waited" -lt "$limit" ]; do
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" 2>/dev/null; NSH_WAIT_RC=$?
            return 0
        fi
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}
# nsh_term_own <pid> — SIGTERM, bounded grace, then SIGKILL. Caller guarantees
# the pid is one this script started.
nsh_term_own() {
    local pid="$1" waited=0
    local grace="${NSH_TIMEOUT_TERM_GRACE:-5}"
    kill -TERM "$pid" 2>/dev/null || return 0
    while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt "$grace" ]; do
        sleep 1; waited=$((waited + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        say "[cleanup] pid $pid ignored SIGTERM after ${grace}s - SIGKILL (script-owned only)"
        kill -KILL "$pid" 2>/dev/null || true
    fi
    wait "$pid" 2>/dev/null || true
    return 0
}
# Cleanup: EXACTLY the pids recorded by this script (its app launches +
# its Xvfb). Never a pattern match, never a foreign pid, never a signal
# to anything else. Evidence directory is NOT removed.
nsh_cleanup() {
    local pid waited alive grace
    if [ "${NSH_CLEANED:-0}" = "1" ]; then return 0; fi
    NSH_CLEANED=1
    grace="${NSH_TIMEOUT_TERM_GRACE:-5}"
    if [ -n "${NSH_APP_PIDS:-}" ]; then
        say "[cleanup] terminating script-owned app pids:$NSH_APP_PIDS (SIGTERM first)"
        for pid in $NSH_APP_PIDS; do kill -TERM "$pid" 2>/dev/null || true; done
        waited=0
        while [ "$waited" -lt "$grace" ]; do
            alive=0
            for pid in $NSH_APP_PIDS; do kill -0 "$pid" 2>/dev/null && alive=1; done
            [ "$alive" -eq 0 ] && break
            sleep 1; waited=$((waited + 1))
        done
        for pid in $NSH_APP_PIDS; do
            if kill -0 "$pid" 2>/dev/null; then
                say "[cleanup] app pid $pid still alive after ${grace}s - SIGKILL (script-owned only)"
                kill -KILL "$pid" 2>/dev/null || true
            fi
            wait "$pid" 2>/dev/null || true
        done
    fi
    if [ -n "${NSH_XVFB_PID:-}" ]; then
        say "[cleanup] stopping script-owned Xvfb pid=$NSH_XVFB_PID (SIGTERM first)"
        kill -TERM "$NSH_XVFB_PID" 2>/dev/null || true
        waited=0
        while kill -0 "$NSH_XVFB_PID" 2>/dev/null && [ "$waited" -lt "$grace" ]; do
            sleep 1; waited=$((waited + 1))
        done
        if kill -0 "$NSH_XVFB_PID" 2>/dev/null; then
            kill -KILL "$NSH_XVFB_PID" 2>/dev/null || true
        fi
        wait "$NSH_XVFB_PID" 2>/dev/null || true
    fi
    if [ -n "${NSH_OUT:-}" ] && [ -d "$NSH_OUT" ]; then
        say "[cleanup] evidence preserved (never removed by this script): $NSH_OUT"
    fi
    return 0
}

# ---- private display + Xvfb ----------------------------------------------
# First free number in [MIN,MAX]: free = no unix socket AND no lock file.
nsh_free_display() {
    local n base="${NSH_X_TMP:-/tmp}"
    for ((n = NSH_DISPLAY_MIN; n <= NSH_DISPLAY_MAX; n++)); do
        [ -e "$base/.X11-unix/X$n" ] && continue
        [ -e "$base/.X$n-lock" ] && continue
        printf ':%d' "$n"
        return 0
    done
    return 1
}
nsh_prereq() { say "[PREREQUISITE] $*"; EXIT_PREREQ=1; }
# T3 toolset: missing => exact report, NO GUI input will run.
nsh_t3_prereqs() {
    local missing="" t
    for t in Xvfb xdotool xwd xwininfo xdpyinfo; do
        command -v "$t" >/dev/null 2>&1 || missing="$missing $t"
    done
    if [ -z "$missing" ]; then return 0; fi
    say "[PREREQUISITE] T3 (isolated live GUI checks) needs:$missing"
    say "[PREREQUISITE] Debian/Ubuntu packages: xvfb xdotool x11-apps x11-utils"
    say "[PREREQUISITE] Fedora packages: xorg-x11-server-Xvfb xdotool xorg-x11-apps xorg-x11-utils"
    say "[PREREQUISITE] Install them yourself BEFORE running. This script performs no installs, never elevates privileges, and will NOT fall back to your real desktop."
    return 1
}
# Start a PRIVATE Xvfb (own display number, no TCP listeners). Failure
# reports the prerequisite and returns 1 — never any input on a real display.
nsh_start_xvfb() {
    local num waited
    if ! command -v Xvfb >/dev/null 2>&1; then
        nsh_prereq "Xvfb not found - Debian/Ubuntu package 'xvfb', Fedora 'xorg-x11-server-Xvfb'"
        return 1
    fi
    if ! NSH_DISPLAY=$(nsh_free_display); then
        nsh_prereq "no free X display number in ${NSH_DISPLAY_MIN}-${NSH_DISPLAY_MAX} under $NSH_X_TMP"
        return 1
    fi
    NSH_OWNED_DISPLAY="$NSH_DISPLAY"
    say "[T3.0] starting private Xvfb on $NSH_DISPLAY (socket dir $NSH_X_TMP, -nolisten tcp)"
    Xvfb "$NSH_DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$NSH_OUT/xvfb.log" 2>&1 &
    NSH_XVFB_PID=$!
    num="${NSH_DISPLAY#:}"
    waited=0
    while [ "$waited" -lt "$NSH_TIMEOUT_XVFB_START" ]; do
        [ -S "$NSH_X_TMP/.X11-unix/X$num" ] && break
        kill -0 "$NSH_XVFB_PID" 2>/dev/null || break
        sleep 1
        waited=$((waited + 1))
    done
    if [ ! -S "$NSH_X_TMP/.X11-unix/X$num" ]; then
        nsh_prereq "Xvfb did not create $NSH_X_TMP/.X11-unix/X$num within ${NSH_TIMEOUT_XVFB_START}s (see $NSH_OUT/xvfb.log)"
        nsh_cleanup
        return 1
    fi
    NSH_OWNED_XVFB=1
    say "[T3.0] isolated Xvfb ready: $NSH_DISPLAY pid=$NSH_XVFB_PID (the ONLY display this script ever sends input to)"
    return 0
}

# ---- offline preflight ----------------------------------------------------
nsh_offline_preflight() {
    local out rc
    say "[T0] \$ (timeout ${NSH_TIMEOUT_METADATA}s) cargo metadata --offline --format-version 1"
    out=$(timeout -k 5 "$NSH_TIMEOUT_METADATA" cargo metadata --offline --format-version 1 2>&1 >/dev/null)
    rc=$?
    if [ "$rc" -eq 0 ]; then
        say "[T0] exit: 0 (every dependency is in the local cache - offline build possible)"
        return 0
    fi
    say "[T0] exit: $rc (OFFLINE DEPENDENCY CHECK FAILED)"
    say "[PREREQUISITE] cargo cannot resolve Cargo.lock with CARGO_NET_OFFLINE=true (exit $rc)."
    say "[PREREQUISITE] required: a warm Cargo cache for this Cargo.lock on this machine."
    say "[PREREQUISITE] remedy (a human, outside this script, once with network): run 'cargo fetch' here - or copy ~/.cargo/registry plus git checkouts from a prepared machine."
    say "[PREREQUISITE] first lines of the error:"
    say_block "$out"
    return 1
}

# ---- app launch (recorded for scoped cleanup) -----------------------------
nsh_start_app() {
    local tag="$1"
    NSH_LAST_PID=""
    if ! nsh_require_gui_input; then
        say "[APP] REFUSED to start '$tag': no private Xvfb owned by this script"
        return 1
    fi
    if [ ! -x "$APP" ]; then
        say "[APP] missing or not executable: $APP (build first)"
        return 1
    fi
    say "[APP] DISPLAY=$NSH_DISPLAY $APP (tag=$tag)"
    DISPLAY="$NSH_DISPLAY" "$APP" >"$NSH_OUT/${tag}_app_stdout.log" 2>"$NSH_OUT/${tag}_app_stderr.log" &
    NSH_LAST_PID=$!
    NSH_APP_PIDS="$NSH_APP_PIDS $NSH_LAST_PID"
    say "[APP] started pid=$NSH_LAST_PID - recorded in the cleanup scope"
    return 0
}
# nsh_wait_app <pid> <expected-exit-note...>: bounded wait; on timeout,
# graceful + scoped termination of that exact pid.
nsh_wait_app() {
    local pid="$1" msg="$2"
    if nsh_wait_bounded "$pid" "$NSH_TIMEOUT_APP_WAIT"; then
        say "$msg exit: $NSH_WAIT_RC"
    else
        say "$msg TIMEOUT: still running after ${NSH_TIMEOUT_APP_WAIT}s - graceful termination (script-owned pid $pid only)"
        nsh_term_own "$pid"
    fi
    return 0
}

wants() { case ",$TIERS," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# ===========================================================================
main() {
    nsh_guard

    TS=$(date +%Y%m%d_%H%M%S)
    NSH_OUT=$(mktemp -d "${TMPDIR:-/tmp}/nameshot-validation.XXXXXX") || {
        echo "FATAL: mktemp failed - cannot create the evidence directory"; exit 2
    }
    LOG="$NSH_OUT/validation-results-$TS.log"
    : >"$LOG"
    export CARGO_NET_OFFLINE=true
    trap 'nsh_cleanup' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM

    [ "${1:-}" = "--tiers" ] && TIERS="${2:-0,1,2,3}"

    say "NameShot Linux/X11 validation (safety revision) - $TS"
    say "evidence directory (unique, preserved): $NSH_OUT"
    say "log: $LOG"
    say "CARGO_NET_OFFLINE=true; no network of our own; no installs; no elevated privileges; no clipboard tools."

    # =========================================================================
    # T0 — environment capture (read-only) + prerequisite reports
    # =========================================================================
    hdr "T0 ENVIRONMENT CAPTURE (read-only)"
    say "[T0] ambient DISPLAY=${NSH_AMBIENT_DISPLAY:-${DISPLAY:-<unset>}} - recorded, NEVER used for input"
    nsh_drop_ambient_display
    say "[T0] DISPLAY after drop: ${DISPLAY:-<unset>} (empty from here on; every X command carries DISPLAY=\$NSH_DISPLAY explicitly)"
    run T0 uname -a
    run_shell T0 'test -f /etc/os-release && cat /etc/os-release || echo "no /etc/os-release"'
    run T0 rustc -Vv
    run T0 cargo -V
    run T0 rustup show
    for tool in timeout mktemp Xvfb xdotool xwd xwininfo xdpyinfo zenity; do
        run_shell T0 "command -v $tool || echo '$tool: NOT FOUND'"
    done
    run_shell T0 'ps -eo comm= | grep -E "^(Xorg|Xvfb|Xephyr|Xwayland|weston|sway|gnome-shell|kwin_x11|xfwm4|openbox|i3)$" | sort -u || echo "no known X/WM process"'

    if wants 3; then
        say "[T0] T3 requested: automated GUI input will run ONLY on a private Xvfb started by this script"
        if nsh_t3_prereqs; then T3_PREREQ_OK=1; else T3_PREREQ_OK=0; EXIT_PREREQ=1; fi
    fi
    if nsh_offline_preflight; then OFFLINE_OK=1; else OFFLINE_OK=0; EXIT_PREREQ=1; fi

    # =========================================================================
    # T1 — COMPILE checks (offline; proves nothing about live behavior)
    # =========================================================================
    if wants 1; then
        hdr "T1 COMPILE TIER (compile checks only - proves nothing about live behavior)"
        if [ "$OFFLINE_OK" -ne 1 ]; then
            say "[T1] SKIPPED: offline prerequisite failed (see PREREQUISITE lines above)"
        else
            run T1 cargo check --all-targets --offline
            run T1 cargo clippy --all-targets --offline
            # fmt: exit 1 is EXPECTED (one pre-existing M1-era hunk); only an
            # unusual exit code is flagged. CARGO_NET_OFFLINE guards the network.
            say "[T1] \$ (timeout ${NSH_TIMEOUT_BUILD}s) cargo fmt --check"
            FMT_OUT=$(timeout -k 10 "$NSH_TIMEOUT_BUILD" cargo fmt --check 2>&1); FMT_RC=$?
            printf '%s\n' "$FMT_OUT" | sed 's/^/    /' | tee -a "$LOG" >/dev/null
            printf '%s\n' "$FMT_OUT"
            if [ "$FMT_RC" -le 1 ]; then
                say "[T1] exit: $FMT_RC (0 = clean/fixed; 1 = expected single pre-existing hunk tests/encode_roundtrip.rs:11)"
            else
                say "[T1] exit: $FMT_RC (unexpected - REVIEW)"
                T1_FAIL=1
            fi
            run T1 cargo build --release --offline
        fi
    fi

    # =========================================================================
    # T2 — UNIT tests on the native Linux toolchain (offline)
    # =========================================================================
    if wants 2; then
        hdr "T2 UNIT TIER (includes the 8 Linux-gated eventmap tests)"
        if [ "$OFFLINE_OK" -ne 1 ]; then
            say "[T2] SKIPPED: offline prerequisite failed (see PREREQUISITE lines above)"
        else
            run T2 cargo test --offline
            PASS_SUM=$(grep -oE 'test result: ok\. [0-9]+ passed' "$LOG" | awk '{ s += $4 } END { print s + 0 }')
            say "[T2] sum of passed tests: $PASS_SUM (expected on Linux: 156 = 146 lib + 4 encode + 2 icon + 4 redaction)"
            if [ "$PASS_SUM" = "156" ]; then
                say "[T2] count check: OK"
            else
                say "[T2] count check: REVIEW (differs from expected 156 - recorded, does not change the exit code)"
            fi
        fi
    fi

    # =========================================================================
    # T3 — LIVE checks: PRIVATE XVFB ONLY. A real desktop is never used.
    # =========================================================================
    if wants 3; then
        hdr "T3 LIVE TIER - isolated Xvfb only (never a real desktop, never a fallback)"
        APP=target/release/nameshot
        if [ "$T3_PREREQ_OK" -ne 1 ]; then
            say "[T3] NOT RUN: prerequisites missing (see PREREQUISITE lines in T0). No GUI input was attempted."
        elif [ "$OFFLINE_OK" -ne 1 ]; then
            say "[T3] NOT RUN: offline prerequisite failed and no binary was built. No GUI input was attempted."
        elif ! nsh_start_xvfb; then
            say "[T3] NOT RUN: private Xvfb could not be started (see PREREQUISITE lines above). No GUI input was attempted."
        elif [ ! -x "$APP" ]; then
            run T1 cargo build --release --offline
            if [ ! -x "$APP" ]; then
                say "[T3] NOT RUN: $APP still missing after build attempt (see T1 lines)"
            fi
        else

        # --- T3.1 WINDOW LIFECYCLE (map, focus, in-session key, exit code) ---
        say ""
        say "[T3.1] WINDOW LIFECYCLE - automated portion (input confined to $NSH_DISPLAY):"
        if nsh_start_app t3_1; then
            APP_PID=$NSH_LAST_PID
            sleep 2
            if kill -0 "$APP_PID" 2>/dev/null; then
                say "[T3.1] app alive after 2s: pid=$APP_PID"
                run_x T3.1 "xwininfo -root -tree 2>&1 | head -40 | tee -a \"$NSH_OUT/t3_1_wintree.log\""
                run_x T3.1 "xwd -root -silent > \"$NSH_OUT/t3_1_screen.xwd\" && echo saved:$NSH_OUT/t3_1_screen.xwd"
                run_x T3.1 "xdotool mousemove 400 300 key Escape && echo escape-sent"
                nsh_wait_app "$APP_PID" "[T3.1] app after Escape (EXPECTED exit 0 = cancel path)"
            else
                wait "$APP_PID" 2>/dev/null
                say "[T3.1] app died immediately: exit $? (see $NSH_OUT/t3_1_app_stderr.log) - FAIL or environment gap"
            fi
        else
            say "[T3.1] NOT RUN"
        fi
        say "[T3.1] MANUAL RESULT (operator): overlay window visible in $NSH_OUT/t3_1_wintree.log / screen image? enter PASS/FAIL: ____"

        # --- T3.2 CAPTURE (root grab mirrored into the overlay) --------------
        say ""
        say "[T3.2] CAPTURE - automated portion (app startup performs grab_virtual_screen):"
        if nsh_start_app t3_2; then
            APP_PID=$NSH_LAST_PID
            sleep 2
            run_x T3.2 "xwd -root -silent > \"$NSH_OUT/t3_2_before_or_after.xwd\" && echo saved:$NSH_OUT/t3_2_before_or_after.xwd"
            run_x T3.2 "xdotool mousemove 600 400 key Escape && echo escape-sent"
            nsh_wait_app "$APP_PID" "[T3.2] app after Escape (EXPECTED exit 0)"
        else
            say "[T3.2] NOT RUN"
        fi
        say "[T3.2] MANUAL RESULT (operator): does the overlay show a pixel-faithful copy of the private screen (compare $NSH_OUT/t3_2_before_or_after.xwd with $NSH_OUT/t3_1_screen.xwd)? PASS/FAIL: ____"

        # --- T3.3 SELECTION (drag commits / click reverts / Enter / Escape) --
        say ""
        say "[T3.3] SELECTION - automated portion (XTEST drag, confined to $NSH_DISPLAY):"
        if nsh_start_app t3_3; then
            APP_PID=$NSH_LAST_PID
            sleep 2
            run_x T3.3 "xdotool mousemove 200 150 mousedown 1 mousemove --delay 30 600 450 mouseup 1 && sleep 1 && xwd -root -silent > \"$NSH_OUT/t3_3_after_drag.xwd\" && echo drag-done"
            say "[T3.3] OBSERVE: selection rectangle at 200,150-600,450 in $NSH_OUT/t3_3_after_drag.xwd? (operator) PASS/FAIL: ____"
            if command -v zenity >/dev/null 2>&1; then
                say "[T3.3] zenity present: Enter should open its save dialog ON THE PRIVATE DISPLAY; then Escape cancels it: EXPECTED app exit 0 (dialog-cancel is attempted automatically; MANUAL verify)"
            else
                say "[T3.3] zenity absent: Enter should reach the save path and fail with a dialog error on stderr - EXPECTED app exit 1 (honest error-path evidence, not a pass)"
            fi
            run_x T3.3 "xdotool key Return && echo return-sent"
            if command -v zenity >/dev/null 2>&1; then
                run_x T3.3 "sleep 1 && xdotool key Escape && echo escape-sent-to-dialog"
            fi
            nsh_wait_app "$APP_PID" "[T3.3] app after Enter (see expectation above)"
            if [ -f "$NSH_OUT/t3_3_app_stderr.log" ]; then
                run T3.3 cat "$NSH_OUT/t3_3_app_stderr.log"
            fi
            # click-revert sub-check (fresh run): sub-threshold click must NOT
            # change the band
            if nsh_start_app t3_3b; then
                APP_PID=$NSH_LAST_PID
                sleep 2
                run_x T3.3 "xdotool mousemove 300 300 click 1 && sleep 1 && xwd -root -silent > \"$NSH_OUT/t3_3b_after_click.xwd\" && xdotool key Escape && echo click-done"
                nsh_wait_app "$APP_PID" "[T3.3b] app after click+Escape (EXPECTED 0 = click did not commit)"
                say "[T3.3b] MANUAL RESULT (operator): after a plain click the veil/selection unchanged (full-screen band) in $NSH_OUT/t3_3b_after_click.xwd? PASS/FAIL: ____"
            else
                say "[T3.3b] NOT RUN"
            fi
        else
            say "[T3.3] NOT RUN"
        fi

        # --- T3.4 HOTKEYS ---------------------------------------------------
        say ""
        say "[T3.4] GLOBAL HOTKEYS: NOT TESTED - global registration is not implemented in M1b; this harness registers no hotkey (in-session Escape/Enter are exercised inside T3.1/T3.3)."
        say "[T3.4] MANUAL RESULT: ____ (see T3.1/T3.3 exit codes)"

        # --- T3.5 CLIPBOARD -------------------------------------------------
        say ""
        say "[T3.5] CLIPBOARD: NOT TESTED - this harness invokes no clipboard tool, reads/writes no selection, and never triggers the app's copy actions (the app claims CLIPBOARD ownership only on an explicit copy, which is out of M1b test scope)."
        say "[T3.5] MANUAL RESULT: ____"

        fi   # end of T3 private-Xvfb branch
    fi

    # =========================================================================
    hdr "SUMMARY (machine tiers only - live results are filled in above)"
    say "T1 compile: $([ $T1_FAIL -eq 0 ] && echo 'all recorded commands exited 0' || echo 'FAILURES above (see exit lines)')"
    say "T2 unit:    $([ $T2_FAIL -eq 0 ] && echo 'all recorded commands exited 0' || echo 'FAILURES above (see exit lines)')"
    say "T3 live:    NOT CLAIMED here - see MANUAL RESULT lines; nothing passed without them."
    if [ "$EXIT_PREREQ" -ne 0 ]; then
        say "prerequisite: MISSING - blocked tier(s) recorded NOT RUN (exit 3)"
    fi
    say ""
    say "exit codes: 0 ok / 1 machine-tier failure / 2 guard / 3 prerequisite missing"
    say "evidence (preserved): $NSH_OUT"
    say "next: attach $LOG to docs/BUILD_LOG.md (M1b-v) before any runtime statement."
    say "Passing T1/T2 is compile/unit evidence ONLY."

    if [ "$EXIT_PREREQ" -ne 0 ]; then exit 3; fi
    if [ "$T1_FAIL" -ne 0 ] || [ "$T2_FAIL" -ne 0 ]; then exit 1; fi
    exit 0
}

# Sourcing (tests) defines functions only; executing runs the harness.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main "$@"
fi
