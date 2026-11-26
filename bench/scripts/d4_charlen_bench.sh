#!/usr/bin/env bash
# D4 基准驱动（Value 字符长度缓存：length/index/range/first/last + 改写链）。
#
# 并排同窗交错测量：每轮先 tclsh9.1 后 rtcl 各跑一次（负载 10–20 下
# 只有比值有意义），ROUNDS 轮取中位数。输出每项一行：
#   <case>  rtcl <us>  tclsh91 <us>  ratio <x>
# us 为 REP（或 chain 类 D4_N 次）的整段墙钟微秒。
#
# 用法：bash bench/scripts/d4_charlen_bench.sh [case...]
#   无参数 = 全部案例。
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RTCL="$ROOT/target/release/rtcl"
TCLSH="$ROOT/.refer/tcl910/inst/bin/tclsh9.1"
ROUNDS="${ROUNDS:-7}"

for bin in "$RTCL" "$TCLSH"; do
    [ -x "$bin" ] || { echo "缺二进制：$bin" >&2; exit 1; }
done

CASES="${*:-len-uni len-ascii idx-end-uni idx-mid-uni rng-uni first-hit-uni last-hit-uni chain-append chain-map chain-trim chain-lower}"

TMP="$(mktemp -d /tmp/rtcl-d4-bench.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
median() { sort -n "$1" | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }

for c in $CASES; do
    FT="$TMP/$c.tcl"; FR="$TMP/$c.rtcl"
    for i in $(seq 1 "$ROUNDS"); do
        us=$(D4_CASE="$c" "$TCLSH" "$ROOT/bench/scripts/d4_charlen_bench.tcl") \
            && echo "$us" >> "$FT"
        us=$(D4_CASE="$c" "$RTCL" -f "$ROOT/bench/scripts/d4_charlen_bench.tcl") \
            && echo "$us" >> "$FR"
    done
    TC=$(median "$FT"); RC=$(median "$FR")
    ratio=$(awk "BEGIN{ if ($TC > 0) printf \"%.2f\", $RC/$TC; else print \"inf\" }")
    printf '%-14s rtcl %-9s tclsh91 %-9s ratio %s\n' "$c" "$RC" "$TC" "$ratio"
done
