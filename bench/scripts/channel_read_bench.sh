#!/usr/bin/env bash
# T6 通道缓冲读基准（HANDOFF §4 T6：P1 TCP 3MiB、P9 文件 32MB）。
#
# 并排同窗交错测量（机器负载常 10–20，比值才有意义）：每轮先 tclsh9.1
# 后 rtcl 各测一次，各 7 轮取中位数。输出每项一行：
#   file-32MB  rtcl <us>  tclsh91 <us>  ratio <x>
#   tcp-3MiB   rtcl <us>  tclsh91 <us>  ratio <x>
#
# 用法：bash bench/scripts/channel_read_bench.sh [worktree 根]
#   默认以脚本位置上两级为仓根；二进制 $ROOT/target/release/rtcl、
#   oracle $ROOT/.refer/tcl910/inst/bin/tclsh9.1。
set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RTCL="$ROOT/target/release/rtcl"
TCLSH="$ROOT/.refer/tcl910/inst/bin/tclsh9.1"
ROUNDS="${ROUNDS:-7}"
TMP="$(mktemp -d /tmp/rtcl-t6-bench.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

for bin in "$RTCL" "$TCLSH"; do
    [ -x "$bin" ] || { echo "缺二进制：$bin" >&2; exit 1; }
done

# 32MB ASCII 文本（行含 \n；utf-8 合法）。
DATA="$TMP/text32m.txt"
if [ ! -s "$DATA" ]; then
    yes "the quick brown fox jumps over the lazy dog 0123456789" | head -c 33554432 > "$DATA"
fi

median() { sort -n "$1" | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }

# --- P9: 文件整读 32MB -------------------------------------------------
F_TCL="$TMP/file.tcl"; F_RTCL="$TMP/file.rtcl"
for i in $(seq 1 "$ROUNDS"); do
    us=$(T6_FILE="$DATA" "$TCLSH" "$ROOT/bench/scripts/channel_read_file_bench.tcl") \
        && echo "$us" >> "$F_TCL"
    us=$(T6_FILE="$DATA" "$RTCL" -f "$ROOT/bench/scripts/channel_read_file_bench.tcl") \
        && echo "$us" >> "$F_RTCL"
done
FT=$(median "$F_TCL"); FR=$(median "$F_RTCL")

# --- P1: TCP 读 3MiB（服务端恒 tclsh9.1，测客户端读）-------------------
P_TCL="$TMP/tcp.tcl"; P_RTCL="$TMP/tcp.rtcl"
PORTFILE="$TMP/port"
for i in $(seq 1 "$ROUNDS"); do
    run_pair() {  # $1 = 客户端命令（数组 caller 上下文）
        rm -f "$PORTFILE"
        T6_PORT_FILE="$PORTFILE" "$TCLSH" "$ROOT/bench/scripts/channel_read_tcp_server.tcl" & local srv=$!
        for _ in $(seq 1 100); do [ -s "$PORTFILE" ] && break; sleep 0.02; done
        [ -s "$PORTFILE" ] || { echo "server 未就绪" >&2; kill $srv 2>/dev/null; return 1; }
        T6_PORT_FILE="$PORTFILE" timeout 30 "$@"
        local rc=$?
        kill $srv 2>/dev/null; wait $srv 2>/dev/null
        return $rc
    }
    us=$(run_pair "$TCLSH" "$ROOT/bench/scripts/channel_read_tcp_client.tcl") \
        && echo "$us" >> "$P_TCL"
    us=$(run_pair "$RTCL" -f "$ROOT/bench/scripts/channel_read_tcp_client.tcl") \
        && echo "$us" >> "$P_RTCL"
done
PT=$(median "$P_TCL"); PR=$(median "$P_RTCL")

awk -v fr="$FR" -v ft="$FT" -v pr="$PR" -v pt="$PT" 'BEGIN {
    printf "file-32MB  rtcl %8d us  tclsh91 %8d us  ratio %.2fx\n", fr, ft, (ft>0? fr/ft : 0)
    printf "tcp-3MiB   rtcl %8d us  tclsh91 %8d us  ratio %.2fx\n", pr, pt, (pt>0? pr/pt : 0)
}'
