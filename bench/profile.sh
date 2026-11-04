#!/usr/bin/env bash
# bench/profile.sh — CPU 剖析（flamegraph）。
#
# 三通道自适应（按序探测，取第一个可用）：
#   A. perf        —— 内核采样，最准。需要 kernel.perf_event_paranoid<=2
#                     （本机常为 4：sudo sysctl -w kernel.perf_event_paranoid=1
#                     或写入 /etc/sysctl.d/ 持久化）。
#   B. gdb poor-man's —— 无 perf 权限时的用户态回退：gdb attach 自家
#                     子进程周期采样回溯（yama ptrace_scope=1 下合法），
#                     负载需运行 ≥ 采样窗口（脚本自动按 -r 重复次数拉长）。
#   C. 提示         —— 两者皆缺时输出解锁指引。
#
# 前置（符号）：与 profile-heap.sh 同——
#   CARGO_PROFILE_RELEASE_STRIP=none cargo build --release -p rtcl-cli
#
# 用法：
#   bash bench/profile.sh [-w workload.tcl] [-o outdir] [-r repeats] [-d secs]
# 产物（$OUT，默认 bench/profile-out/cpu）：
#   cpu_flame.svg    火焰图
#   cpu_collapsed.txt 折叠栈（可再加工/对比 diff 火焰图）
#   cpu_perf.data    （A 通道）原始 perf 数据，可用 perf report 复查
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/rtcl
TOOLS=bench/tools
OUT=bench/profile-out/cpu
WORKLOAD=""
REPEATS=1
DURATION=8
while getopts "w:o:r:d:" opt; do
    case $opt in
        w) WORKLOAD=$OPTARG ;;
        o) OUT=$OPTARG ;;
        r) REPEATS=$OPTARG ;;
        d) DURATION=$OPTARG ;;
        *) echo "usage: $0 [-w workload.tcl] [-o outdir] [-r repeats] [-d secs]" >&2; exit 2 ;;
    esac
done
mkdir -p "$OUT"
if ! file "$BIN" 2>/dev/null | grep -q 'not stripped'; then
    echo "cpu-profile: $BIN 已 strip——重建：CARGO_PROFILE_RELEASE_STRIP=none cargo build --release -p rtcl-cli" >&2
    exit 2
fi

# ── 负载包装：-r 重复拉长采样窗口（poor-man's 需要）───────────────
if [ -z "$WORKLOAD" ]; then
    WORKLOAD=bench/cases/arith_loop.tcl
fi
WRAP=$OUT/profile_wrap.tcl
{
    echo "for {set __i 0} {\$__i < $REPEATS} {incr __i} {"
    echo "    source [file join [pwd] $WORKLOAD]"
    echo "}"
} > "$WRAP"

# ── A. perf ───────────────────────────────────────────────────────
if command -v perf >/dev/null 2>&1; then
    if perf record -q -e cycles:u -o "$OUT/cpu_perf.data" -- \
        "$BIN" -f "$WRAP" >/dev/null 2>&1; then
        perf script -i "$OUT/cpu_perf.data" \
            | perl "$TOOLS/stackcollapse-perf.pl" > "$OUT/cpu_collapsed.txt"
        perl "$TOOLS/flamegraph.pl" --countname samples \
            < "$OUT/cpu_collapsed.txt" > "$OUT/cpu_flame.svg"
        echo "cpu-profile[perf]: $OUT/cpu_flame.svg  $OUT/cpu_collapsed.txt  $OUT/cpu_perf.data"
        head -12 "$OUT/cpu_collapsed.txt" | sort -t';' -k2 || true
        exit 0
    fi
    echo "cpu-profile: perf 不可用（perf_event_paranoid=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null)）——回落 gdb poor-man's" >&2
fi

# ── B. gdb SIGABRT 采样 ───────────────────────────────────────────
# yama ptrace_scope=1 友好：gdb 是目标进程的父（launch 采样合法），
# 每轮 run → 外部 kill -ABRT → gdb 批处理打全栈 → N 轮折叠成火焰图。
# 负载时长须 > ABRT_DELAY（-r 拉长；过早退出则轮次空转自动跳过）。
if command -v gdb >/dev/null 2>&1; then
    SAMPLES="${PM_SAMPLES:-30}"
    DELAY="${ABRT_DELAY:-1.2}"
    RAW=$OUT/pm_stacks.raw
    : > "$RAW"
    ok=0
    for _ in $(seq "$SAMPLES"); do
        gdb -batch \
            -ex 'handle SIGABRT stop nopass' \
            -ex 'run' \
            -ex 'thread apply all bt' \
            --args "$BIN" -f "$WRAP" > "$OUT/pm_run.log" 2>&1 &
        GPID=$!
        TPID=""
        for _ in $(seq 80); do
            TPID=$(ps --ppid "$GPID" -o pid=,comm= 2>/dev/null \
                | awk '$2 == "rtcl" { print $1; exit }' || true)
            [ -n "$TPID" ] && break
            sleep 0.05
        done
        if [ -n "$TPID" ]; then
            sleep "$DELAY"
            kill -ABRT "$TPID" 2>/dev/null || true
            sleep 0.4
        fi
        wait "$GPID" 2>/dev/null || true
        if grep -q '^#0' "$OUT/pm_run.log" 2>/dev/null; then
            cat "$OUT/pm_run.log" >> "$RAW"
            ok=$((ok + 1))
        fi
    done
    if [ "$ok" -gt 0 ]; then
        # 折叠：#N 帧 → 分号链，一行一样本（rtcl 符号优先，匿名/内核帧跳过）。
        perl -e '
            my %t;
            my ($cur, $n) = ("", 0);
            open(my $fh, "<", $ARGV[0]) or die;
            my @lines = <$fh>;
            for my $line (@lines) {
                chomp $line;
                if ($line =~ /^#\d+\s+(?:0x[0-9a-f]+\s+in\s+)?([^\s(]+)/) {
                    my $f = $1;
                    next if $f eq "??" || $f =~ /^__/;
                    $cur = "$f;$cur";
                    $n++;
                } elsif ($n) {
                    $t{$cur}++;
                    ($cur, $n) = ("", 0);
                }
            }
            $t{$cur}++ if $n;
            print "$_ $t{$_}\n" for sort { $t{$b} <=> $t{$a} } keys %t;
        ' "$RAW" > "$OUT/cpu_collapsed.txt"
        perl "$TOOLS/flamegraph.pl" --countname samples \
            < "$OUT/cpu_collapsed.txt" > "$OUT/cpu_flame.svg"
        echo "cpu-profile[gdb-sigabrt]: $OUT/cpu_flame.svg  $OUT/cpu_collapsed.txt（$ok/$SAMPLES 轮有效）"
        head -8 "$OUT/cpu_collapsed.txt"
        exit 0
    fi
    echo "cpu-profile: gdb SIGABRT 采样无产出（负载短于 ABRT_DELAY？调 -r/ABRT_DELAY）" >&2
fi

# ── C. 指引 ───────────────────────────────────────────────────────
echo "cpu-profile: 无可用采样通道。任选其一：" >&2
echo "  sudo sysctl -w kernel.perf_event_paranoid=1   # 解锁 perf（A 通道）" >&2
echo "  sudo apt install valgrind                     # 或装 callgrind" >&2
exit 3
