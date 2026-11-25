#!/usr/bin/env bash
# T7 基准驱动（P5 array、数组元素读写、P15 string last 三处）。
#
# 并排同窗交错测量：每轮先 tclsh9.1 后 rtcl 各跑一次（负载 10–20 下
# 只有比值有意义），ROUNDS 轮取中位数。输出每项一行：
#   <case>  rtcl <us>  tclsh91 <us>  ratio <x>
# us 为整段墙钟微秒（read-50k 等 = 5 万次操作的整段；size/exists/
# string 各案例为 REP 次的整段）。
#
# 用法：bash bench/scripts/t7_bench.sh [array|string] [case...]
#   无参数 = 全部案例。
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RTCL="$ROOT/target/release/rtcl"
TCLSH="$ROOT/.refer/tcl910/inst/bin/tclsh9.1"
ROUNDS="${ROUNDS:-7}"

for bin in "$RTCL" "$TCLSH"; do
    [ -x "$bin" ] || { echo "缺二进制：$bin" >&2; exit 1; }
done

ARRAY_CASES="size-80k exists-80k exists-miss read-50k write-new write-old micro-write micro-read micro-incr micro-append"
STRING_CASES="last-hit last-miss-sp last-hit-uni"

if [ "${1:-}" = "string" ]; then
    SCRIPT="t7_string_bench.tcl"; CASES="${2:-$STRING_CASES}"
else
    SCRIPT="t7_array_bench.tcl"; CASES="${2:-$ARRAY_CASES}"
fi

TMP="$(mktemp -d /tmp/rtcl-t7-bench.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
median() { sort -n "$1" | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }

for c in $CASES; do
    FT="$TMP/$c.tcl"; FR="$TMP/$c.rtcl"
    for i in $(seq 1 "$ROUNDS"); do
        us=$(T7_CASE="$c" "$TCLSH" "$ROOT/bench/scripts/$SCRIPT") \
            && echo "$us" >> "$FT"
        us=$(T7_CASE="$c" "$RTCL" -f "$ROOT/bench/scripts/$SCRIPT") \
            && echo "$us" >> "$FR"
    done
    TC=$(median "$FT"); RC=$(median "$FR")
    ratio=$(awk "BEGIN{ if ($TC > 0) printf \"%.2f\", $RC/$TC; else print \"inf\" }")
    printf '%-14s rtcl %-9s tclsh91 %-9s ratio %s\n' "$c" "$RC" "$TC" "$ratio"
done
