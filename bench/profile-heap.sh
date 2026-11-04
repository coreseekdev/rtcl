#!/usr/bin/env bash
# bench/profile-heap.sh — jemalloc 堆剖析（内存分析）。
#
# 前置：rtcl 以剖析分配器构建（bench/PROFILE.md）：
#   CARGO_PROFILE_RELEASE_STRIP=none cargo build --release --features jemalloc -p rtcl-cli
# （STRIP=none 保留符号——jeprof 解析调用栈必需；默认 release 是
#   strip = true，剖析构建必须覆写。）
#
# 用法：
#   bash bench/profile-heap.sh                     # 内置保留型负载
#   bash bench/profile-heap.sh -w my.tcl -o out    # 指定负载/输出目录
#   bash bench/profile-heap.sh -c -f bench/cases/fib25.tcl   # 原样传给 rtcl
#
# 产物（$OUT，默认 bench/profile-out/heap）：
#   jeprof.*.heap    堆快照（退出时 + 期间按 lg_prof_interval 采样）
#   heap_top.txt     jeprof --text（存活字节 TopN）
#   heap_flame.svg   火焰图（jeprof --collapsed | flamegraph.pl）
#
# 环境变量：MALLOC_PROF_OPTS 覆盖 MALLOC_CONF 剖析段；
#   JEPROF_FOCUS=rtcl_vm 传给 jeprof --focus（滤掉分配器自身帧）。
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/rtcl
TOOLS=bench/tools
OUT=bench/profile-out/heap
WORKLOAD=""
RTCL_ARGS=()
while getopts "w:o:c" opt; do
    case $opt in
        w) WORKLOAD=$OPTARG ;;
        o) OUT=$OPTARG ;;
        c) shift $((OPTIND - 1)); RTCL_ARGS=("$@"); break ;;
        *) echo "usage: $0 [-w workload.tcl] [-o outdir] [-c rtcl-args...]" >&2; exit 2 ;;
    esac
done
mkdir -p "$OUT"

# ── 构建检查：jemalloc + 未 strip ─────────────────────────────────
if ! grep -q prof_backtrace_impl <(strings "$BIN" 2>/dev/null) 2>/dev/null; then
    echo "heap-profile: $BIN 没有 jemalloc 剖析符号——重建：" >&2
    echo "  CARGO_PROFILE_RELEASE_STRIP=none cargo build --release --features jemalloc -p rtcl-cli" >&2
    exit 2
fi
if ! file "$BIN" | grep -q 'not stripped'; then
    echo "heap-profile: $BIN 已 strip（符号缺失）——用 STRIP=none 重建：" >&2
    echo "  CARGO_PROFILE_RELEASE_STRIP=none cargo build --release --features jemalloc -p rtcl-cli" >&2
    exit 2
fi

# ── 负载：未给 -w 时用内置保留型负载（退出时仍有大量存活分配）──────
if [ -z "$WORKLOAD" ]; then
    WORKLOAD=$OUT/retention_workload.tcl
    mkdir -p "$(dirname "$WORKLOAD")"
    cat > "$WORKLOAD" <<'EOF'
# 保留型负载：30 万双元素 list 挂在 ::l 上直到退出——退出堆快照
# 能看到真实存活结构（rtcl_vm 的 list/value 路径）。
set l {}
for {set i 0} {$i < 300000} {incr i} {
    lappend l [list key$i value$i]
}
puts [llength $l]
EOF
fi

# ── 运行 + 堆快照 ─────────────────────────────────────────────────
rm -f "$OUT"/jeprof.*.heap
CONF="${MALLOC_PROF_OPTS:-prof:true,lg_prof_interval:26,lg_prof_sample:12}"
MALLOC_CONF="$CONF,prof_prefix:$OUT/jeprof" \
    timeout "${HEAP_SECS:-180}" "$BIN" -f "$WORKLOAD" "${RTCL_ARGS[@]}" >/dev/null
DUMPS=$(ls -t "$OUT"/jeprof.*.heap 2>/dev/null | head -1)
[ -n "$DUMPS" ] || { echo "heap-profile: 未产生堆快照（MALLOC_CONF 是否被传到进程？）" >&2; exit 2; }
echo "heap-profile: 快照 $(ls "$OUT"/jeprof.*.heap | wc -l) 份，最新 $DUMPS"

# ── 报表 ──────────────────────────────────────────────────────────
FOCUS=()
[ -n "${JEPROF_FOCUS:-}" ] && FOCUS=(--focus "$JEPROF_FOCUS")
jeprof --show_bytes --text "${FOCUS[@]}" "$BIN" "$DUMPS" > "$OUT/heap_top.txt" 2>/dev/null
head -30 "$OUT/heap_top.txt"
if command -v flamegraph.pl >/dev/null 2>&1 || [ -x "$TOOLS/flamegraph.pl" ]; then
    jeprof --collapsed "${FOCUS[@]}" "$BIN" "$DUMPS" 2>/dev/null \
        | perl "$TOOLS/flamegraph.pl" --countname bytes \
        > "$OUT/heap_flame.svg"
    echo "heap-profile: $OUT/heap_top.txt  $OUT/heap_flame.svg"
else
    echo "heap-profile: $OUT/heap_top.txt（flamegraph.pl 缺失，SVG 跳过）"
fi
