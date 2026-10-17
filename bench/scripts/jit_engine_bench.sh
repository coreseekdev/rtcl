#!/usr/bin/env bash
# M1' gate: bench/cases/arith_loop.tcl — interp vs jit (wasmi) vs jit
# (wasmtime) best-of-N wall clock（TASK-JIT-WASM.md §2 验收线 ≥3×）。
# 用法: bench/scripts/jit_engine_bench.sh [case.tcl] [reps]
set -u
cd "$(dirname "$0")/../.."
CASE=${1:-bench/cases/arith_loop.tcl}
REPS=${2:-5}
RTCL=./target/release/rtcl
JIT=./target/release/jitsh

best() { local best= ; for _ in $(seq "$REPS"); do
  local s e ms; s=$(date +%s%N); "$@" >/dev/null 2>&1; e=$(date +%s%N)
  ms=$(( (e-s)/1000000 )); [ -z "$best" ] || [ "$ms" -lt "$best" ] && best=$ms
done; echo "$best"; }

i=$(best $RTCL -f $CASE)
w=$(best $JIT -f $CASE)
t=$(best env RTCL_JIT_ENGINE=wasmtime $JIT -f $CASE)
echo "case=$CASE reps=$REPS"
echo "interp=$i ms  jit-wasmi=$w ms ($(echo "scale=2; $i / $w" | bc)x)  jit-wasmtime=$t ms ($(echo "scale=2; $i / $t" | bc)x)"
