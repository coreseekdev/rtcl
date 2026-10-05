#!/usr/bin/env bash
# two-engine differential sweep: bytecode vs tree-walk (rtcl vs rtcl
# --no-bytecode).  Every probe must agree byte-exactly between the
# engines — this is the standing gate for any executor-side change.
#
# Excluded: pw/pw2/bnprobe2/objmech/objsec (interactive/security perf
# scripts), objtime/objquad (they PRINT measured wall time — an
# engine-vs-engine diff there is load noise, not semantics; objquad's
# 80k list-nest loop sits near its 1s print boundary and flapped 5/8
# rounds under sustained load, both directions, 2026-10-05).
#
# The tree arm is selected by the CLI --no-bytecode FLAG, not the
# RTCL_NO_BYTECODE env var — the env var materialises into $env(*) and
# polluted every variable-enumeration probe.
set -u
cd "$(dirname "$0")/.."
RTCL=${RTCL:-./target/release/rtcl}
diffs=0
for f in judge/probes/*.tcl; do
    b=$(basename "$f" .tcl)
    case "$b" in
        pw|pw2|bnprobe2|objmech|objsec|objtime|objquad) continue;;
    esac
    a=$(timeout 120 "$RTCL" -f "$f" 2>&1; echo "EXIT=$?")
    e=$(timeout 120 "$RTCL" --no-bytecode -f "$f" 2>&1; echo "EXIT=$?")
    if [ "$a" != "$e" ]; then
        echo "DIFF: $b"
        diffs=$((diffs+1))
    fi
done
echo "sweep: total-diffs: $diffs"
[ "$diffs" -eq 0 ]
