#!/usr/bin/env bash
# bench: performance baseline — rtcl vs tclsh
#
# Usage: bench/run.sh [reps]     (default 3 reps, reports best-of)
set -u
cd "$(dirname "$0")/.."
REPS=${1:-3}
RTCL=./target/release/rtcl

if [ ! -x "$RTCL" ]; then
    echo "bench: build first: cargo build --release -p rtcl-cli" >&2; exit 2
fi

run_best() { # engine cmd... -> best ms on stdout
    local engine="$1" file="$2" best=""
    for _ in $(seq "$REPS"); do
        local start end ms
        start=$(date +%s%N)
        if [ "$engine" = tclsh ]; then tclsh "$file" >/dev/null 2>&1; else "$RTCL" -f "$file" >/dev/null 2>&1; fi
        end=$(date +%s%N)
        ms=$(( (end - start) / 1000000 ))
        if [ -z "$best" ] || [ "$ms" -lt "$best" ]; then best=$ms; fi
    done
    echo "$best"
}

printf "%-16s %10s %10s %8s\n" case tclsh_ms rtcl_ms ratio
total_ratio=0; n=0
for f in bench/cases/*.tcl; do
    name=$(basename "$f" .tcl)
    t=$(run_best tclsh "$f")
    r=$(run_best rtcl "$f")
    if [ "$t" -gt 0 ]; then ratio=$(echo "scale=1; $r / $t" | bc); else ratio="inf"; fi
    printf "%-16s %10s %10s %8s\n" "$name" "$t" "$r" "$ratio"
done
