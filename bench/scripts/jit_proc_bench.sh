#!/usr/bin/env bash
# M2 seam 性能：proc_fib（调用密集体）interp vs jit。
# 已知形状：宿主 eval 路由（[fib …] 括号走完整 eval 链）吃掉调用密集
# 体的收益——真实提速需 M2 全案的 CallProc 原生直连 + slot 单态缓存。
# 本脚本的作用 = 钩子生效性证明（路径已进 JIT）+ 数字台账。
set -u
cd "$(dirname "$0")/../.."
CASE=${1:-bench/cases/proc_fib.tcl}
REPS=${2:-5}
best() { local best= ; for _ in $(seq "$REPS"); do
  local s e ms; s=$(date +%s%N); "$@" >/dev/null 2>&1; e=$(date +%s%N)
  ms=$(( (e-s)/1000000 )); [ -z "$best" ] || [ "$ms" -lt "$best" ] && best=$ms
done; echo "$best"; }
i=$(best ./target/release/rtcl -f $CASE)
j=$(best ./target/release/jitsh -f $CASE)
echo "case=$CASE reps=$REPS interp=${i}ms jit(hook)=${j}ms ratio=$(echo "scale=2; $i / $j" | bc)x"
