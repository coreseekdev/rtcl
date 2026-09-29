#!/usr/bin/env bash
# judge: tclsh vs rtcl parity harness
#
# Runs every script in judge/corpus/ under both tclsh (oracle) and rtcl,
# then byte-diffs stdout + exit code. Exit 0 iff all scripts match.
#
# Usage:
#   judge/run.sh                 # full run, prints burndown table
#   judge/run.sh -v              # also print diffs for failures
#   judge/run.sh <name>          # run a single corpus script
#
# Adding a case: drop a .tcl file into judge/corpus/. Scripts must be
# deterministic (no clock/random) and self-contained.
set -u
cd "$(dirname "$0")/.."

RTCL=./target/release/rtcl
ORACLE=tclsh
VERBOSE=0
[ "${1:-}" = "-v" ] && { VERBOSE=1; shift; }

if [ ! -x "$RTCL" ]; then
    echo "judge: $RTCL not found; run 'cargo build --release -p rtcl-cli' first" >&2
    exit 2
fi

pass=0; fail=0; skip=0; failed=()
files=$(find judge/corpus -name "$(basename "${1:-*}").tcl" | sort)
[ -z "$files" ] && { echo "judge: no corpus files matched" >&2; exit 2; }

for f in $files; do
    name=${f#judge/corpus/}; name=${name%.tcl}
    # Skip marker: a first-line comment `# judge: skip` for known-divergent cases
    if head -1 "$f" | grep -q '^# judge: skip'; then
        skip=$((skip+1)); continue
    fi
    o_out=$(timeout 15 "$ORACLE" "$f" 2>&1); o_code=$?
    r_out=$(timeout 15 "$RTCL" -f "$f" 2>&1); r_code=$?
    if [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$name")
        if [ "$VERBOSE" = 1 ]; then
            echo "=== FAIL: $name ==="
            diff <(printf '%s' "$o_out") <(printf '%s' "$r_out") | head -20
            [ "$o_code" != "$r_code" ] && echo "  exit: oracle=$o_code rtcl=$r_code"
        fi
    fi
done

echo
echo "judge: pass=$pass fail=$fail skip=$skip"
[ ${#failed[@]} -gt 0 ] && echo "failed: ${failed[*]}"
[ "$fail" = 0 ]
