#!/usr/bin/env bash
# probe runner: tclsh-vs-rtcl parity over the oracle-probe archive
#
# judge/probes/*.tcl are the scratch scripts used while diagnosing
# divergences (expr lexing, switch semantics, dict with, command traces,
# ...). Each one pins behaviors that were probed against tclsh 8.6.17
# before being implemented; keep them passing. Files whose expected
# output involves ::errorInfo frame chains are known-divergent until the
# errorInfo batch lands — they print DIFF on purpose.
#
# Usage: bash judge/probes.sh [name-prefix]
set -u
cd "$(dirname "$0")/.."

RTCL=./target/release/rtcl
ORACLE=tclsh
[ ! -x "$RTCL" ] && { echo "probes: $RTCL not found; cargo build --release first" >&2; exit 2; }

pass=0; fail=0; failed=()
for f in judge/probes/${1:-*}.tcl; do
    o_out=$(timeout 15 "$ORACLE" "$f" 2>&1); o_code=$?
    r_out=$(timeout 15 "$RTCL" -f "$f" 2>&1); r_code=$?
    if [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$(basename "$f")")
    fi
done

echo "probes: pass=$pass fail=$fail"
if [ "$fail" -gt 0 ]; then
    printf '  DIFF: %s\n' "${failed[@]}"
    exit 1
fi
