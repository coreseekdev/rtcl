#!/usr/bin/env bash
# compare_tcl.sh — rtcl vs tclsh 全面对比（指令数 + wall，best-of-N）。
#
# 用法：
#   bash bench/compare_tcl.sh [REPS]            # 默认 3 次
#   INSTR=0 bash bench/compare_tcl.sh 5         # 跳过指令数（无 perf 权限时）
#
# 覆盖：
#   1) bench/scripts/*.tcl         — 基准套件（fib/fe/dloop/oo/oovars/sum_only）
#   2) judge/corpus 全量           — 88 个差分语料作「广谱工作负载」，测总耗时/总指令
#   3) 计算型 micro 矩阵           — 空循环/incr/加和/lappend 的增量分解
#
# 指令比（rtcl/tclsh）是主指标（wasm 视角）；wall 用交错 best-of-N。
# 需要：perf（instructions:u）与 setarch -R；无 perf 时自动退化为仅 wall。
set -u
cd "$(dirname "$0")/.."

REPS=${1:-3}
RTCL=./target/release/rtcl
ORACLE=${ORACLE:-tclsh}
INSTR=${INSTR:-1}
[ -x "$RTCL" ] || { echo "compare: $RTCL 不存在；先 cargo build --release -p rtcl-cli" >&2; exit 2; }

perf_ok=0
if [ "$INSTR" = 1 ] && command -v perf >/dev/null 2>&1; then
  if setarch "$(uname -m)" -R perf stat -e instructions:u true >/dev/null 2>&1; then
    perf_ok=1
  fi
fi

best_ms() { # best_ms <engine-cmd...> <script>
  local best=999999999
  for _ in $(seq "$REPS"); do
    local t0 t1
    t0=$(date +%s%N)
    "$@" >/dev/null 2>&1
    t1=$(date +%s%N)
    local ms=$(( (t1 - t0) / 1000000 ))
    [ "$ms" -lt "$best" ] && best=$ms
  done
  echo "$best"
}

instr_of() { # instr_of <engine-cmd...> <script>
  if [ "$perf_ok" = 0 ]; then echo "-"; return; fi
  setarch "$(uname -m)" -R perf stat -e instructions:u "$@" 2>&1 \
    | grep -oE '[0-9,]+ +instructions:u' | tr -d ', ' | sed 's/instructions:u//'
}

ratio() { # ratio <rtcl> <tclsh> -> x.xx
  if [ "$1" = "-" ] || [ "$2" = "-" ] || [ -z "$1" ] || [ -z "$2" ]; then echo "-"; return; fi
  python3 -c "print(f'{$1/$2:.2f}x')"
}

printf '== rtcl vs %s（指令数主指标，wall best-of-%d）==\n\n' "$ORACLE" "$REPS"
printf '| workload | rtcl instr | tclsh instr | instr ratio | rtcl wall(ms) | tclsh wall(ms) | wall ratio |\n'
printf '|---|---|---|---|---|---|---|\n'

total_r=0; total_t=0; total_tw=0; total_rw=0

for f in bench/scripts/fib25.tcl bench/scripts/fe_bench.tcl bench/scripts/dloop.tcl \
         bench/scripts/oo_bench2.tcl bench/scripts/oo_bench2_vars.tcl bench/scripts/sum_only.tcl; do
  [ -f "$f" ] || continue
  ri=$(instr_of "$RTCL" -f "$f"); ti=$(instr_of "$ORACLE" "$f")
  rw=$(best_ms "$RTCL" -f "$f");  tw=$(best_ms "$ORACLE" "$f")
  printf '| %s | %s | %s | %s | %s | %s | %s |\n' \
    "$(basename "$f" .tcl)" "$ri" "$ti" "$(ratio "$ri" "$ti")" "$rw" "$tw" "$(ratio "$rw" "$tw")"
done

# ── 广谱：judge/corpus 全量（语义等价的工作负载集合） ─────────────────
corpus=$(find judge/corpus -name '*.tcl' | sort)
n=$(echo "$corpus" | wc -l)
if [ "$perf_ok" = 1 ]; then
  ri=$(setarch "$(uname -m)" -R perf stat -e instructions:u bash -c \
        "for f in $corpus; do $RTCL -f \"\$f\" >/dev/null 2>&1; done" 2>&1 \
        | grep -oE '[0-9,]+ +instructions:u' | tr -d ', ' | sed 's/instructions:u//')
  ti=$(setarch "$(uname -m)" -R perf stat -e instructions:u bash -c \
        "for f in $corpus; do $ORACLE \"\$f\" >/dev/null 2>&1; done" 2>&1 \
        | grep -oE '[0-9,]+ +instructions:u' | tr -d ', ' | sed 's/instructions:u//')
else
  ri=-; ti=-
fi
tw=999999999; rw=999999999
for _ in $(seq "$REPS"); do
  t0=$(date +%s%N); for f in $corpus; do $ORACLE "$f" >/dev/null 2>&1; done; t1=$(date +%s%N)
  ms=$(( (t1-t0)/1000000 )); [ "$ms" -lt "$tw" ] && tw=$ms
  t0=$(date +%s%N); for f in $corpus; do $RTCL -f "$f" >/dev/null 2>&1; done; t1=$(date +%s%N)
  ms=$(( (t1-t0)/1000000 )); [ "$ms" -lt "$rw" ] && rw=$ms
done
printf '| judge-corpus(%s files) | %s | %s | %s | %s | %s | %s |\n' \
  "$n" "$ri" "$ti" "$(ratio "$ri" "$ti")" "$rw" "$tw" "$(ratio "$rw" "$tw")"

# ── micro 矩阵（增量分解） ────────────────────────────────────────────
for f in bench/micro/micro_base.tcl bench/micro/micro_incr.tcl \
         bench/micro/micro_add.tcl bench/micro/micro_lappend.tcl; do
  [ -f "$f" ] || continue
  ri=$(instr_of "$RTCL" -f "$f"); ti=$(instr_of "$ORACLE" "$f")
  rw=$(best_ms "$RTCL" -f "$f");  tw=$(best_ms "$ORACLE" "$f")
  printf '| %s | %s | %s | %s | %s | %s | %s |\n' \
    "$(basename "$f" .tcl)" "$ri" "$ti" "$(ratio "$ri" "$ti")" "$rw" "$tw" "$(ratio "$rw" "$tw")"
done
