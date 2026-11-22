#!/usr/bin/env bash
# probe runner: tclsh-vs-rtcl parity over the oracle-probe archive
#
# judge/probes/*.tcl are the scratch scripts used while diagnosing
# divergences (expr lexing, switch semantics, dict with, command traces,
# ...). Each one pins behaviors that were probed against tclsh 8.6.17
# before being implemented; keep them passing. The active set must be
# fully green — any DIFF here is a regression.
#
# judge/probes-known/*.tcl pin behaviors that are deliberately divergent
# (design differences, no valid oracle, theoretical boundaries). Every
# one of them has an entry in specs/DIVERGENCES.md (2026-11-22 section)
# stating the root cause and the reopen condition; they are reported
# separately and do not count toward the fail total.
#
# Usage: bash judge/probes.sh [name-prefix]
set -u
cd "$(dirname "$0")/.."

RTCL=./target/release/rtcl
ORACLE=tclsh
[ ! -x "$RTCL" ] && { echo "probes: $RTCL not found; cargo build --release first" >&2; exit 2; }

run_one() {
    local f=$1
    local o_out r_out o_code r_code
    o_out=$(timeout 15 "$ORACLE" "$f" 2>&1); o_code=$?
    r_out=$(timeout 15 "$RTCL" -f "$f" 2>&1); r_code=$?
    if [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ]; then
        return 0
    fi
    return 1
}

pass=0; fail=0; failed=()
for f in judge/probes/${1:-*}.tcl; do
    if run_one "$f"; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$(basename "$f")")
    fi
done

known_pass=0; known_fail=0; known_failed=()
for f in judge/probes-known/${1:-*}.tcl; do
    [ -e "$f" ] || continue
    if run_one "$f"; then
        known_pass=$((known_pass+1))
    else
        known_fail=$((known_fail+1)); known_failed+=("$(basename "$f")")
    fi
done

echo "probes: pass=$pass fail=$fail (known: pass=$known_pass fail=$known_fail of $((known_pass+known_fail)))"
if [ "$fail" -gt 0 ]; then
    printf '  DIFF: %s\n' "${failed[@]}"
    exit 1
fi
if [ "$known_fail" -gt 0 ]; then
    printf '  KNOWN-DIFF (archived, see specs/DIVERGENCES.md 2026-11-22): %s\n' "${known_failed[@]}"
fi
