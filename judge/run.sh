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
#   judge/run.sh --engine jit    # three-way differential: tclsh vs rtcl
#                                # (interp) vs jitsh (JIT engine) — the M1'
#                                # gate (TASK-JIT-WASM.md §2)
#
# Adding a case: drop a .tcl file into judge/corpus/. Scripts must be
# deterministic (no clock/random) and self-contained.
set -u
cd "$(dirname "$0")/.."

RTCL=./target/release/rtcl
JITSH=./target/release/jitsh
ORACLE=tclsh
VERBOSE=0
ENGINE=interp
if [ "${1:-}" = "--engine" ] && [ "${2:-}" = "jit" ]; then
    ENGINE=jit
    shift 2
fi
[ "${1:-}" = "-v" ] && { VERBOSE=1; shift; }

if [ ! -x "$RTCL" ]; then
    echo "judge: $RTCL not found; run 'cargo build --release -p rtcl-cli' first" >&2
    exit 2
fi
if [ "$ENGINE" = jit ] && [ ! -x "$JITSH" ]; then
    echo "judge: $JITSH not found; run 'cargo build --release -p rtcl-jit --features jit-exec --bin jitsh' first" >&2
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
    if [ "$ENGINE" = jit ]; then
        j_out=$(timeout 15 "$JITSH" -f "$f" 2>&1); j_code=$?
        ok=1
        [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ] || ok=0
        [ "$r_out" = "$j_out" ] && [ "$r_code" = "$j_code" ] || ok=0
    else
        ok=1
        [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ] || ok=0
    fi
    if [ "$ok" = "1" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$name")
        if [ "$VERBOSE" = 1 ]; then
            echo "=== FAIL: $name ==="
            diff <(printf '%s' "$o_out") <(printf '%s' "$r_out") | head -20
            [ "$o_code" != "$r_code" ] && echo "  exit: oracle=$o_code rtcl=$r_code"
            if [ "$ENGINE" = jit ]; then
                diff <(printf '%s' "$r_out") <(printf '%s' "$j_out") | head -20
                [ "$r_code" != "$j_code" ] && echo "  exit: rtcl=$r_code jit=$j_code"
            fi
        fi
    fi
done

echo
echo "judge($ENGINE): pass=$pass fail=$fail skip=$skip"
[ ${#failed[@]} -gt 0 ] && echo "failed: ${failed[*]}"
[ "$fail" = 0 ]
