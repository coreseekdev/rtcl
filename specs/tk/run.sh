#!/usr/bin/env bash
# Tk 对照跑手：同一脚本分别在真 Tk（wish）与 rtcl 的 GTK 后端（tkprobe）
# 下运行，按容差比对（specs/tk/compare.tcl）。
#
# 用法：bash specs/tk/run.sh
#
# 与 specs/scripts/run.sh（tclsh 逐字节比对）分开：这里需要 wish、显示器
# 和容差——字体度量在两套工具包间不可能逐像素相同。Tk 报设备像素
# （Xft.dpi），rtcl 报 96dpi 逻辑像素，比例由 xrdb 的 Xft.dpi 给出。
set -u
cd "$(dirname "$0")/../.."

PROBE=./target/debug/tkprobe
command -v wish >/dev/null || { echo "tk: wish not found" >&2; exit 2; }
[ -x "$PROBE" ] || { echo "tk: $PROBE not found; cargo build -p rtcl-tk-demo --bin tkprobe first" >&2; exit 2; }

dpi=$(xrdb -query 2>/dev/null | awk '/^Xft.dpi:/ {print $2}')
scale=$(awk -v dpi="${dpi:-96}" 'BEGIN { printf "%.4f", dpi / 96 }')

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
timeout 60 wish specs/tk/font-metrics.tcl > "$out/tk.out" 2> "$out/tk.err" \
    || { echo "tk: wish failed" >&2; cat "$out/tk.err" >&2; exit 2; }
timeout 60 "$PROBE" specs/tk/font-metrics.tcl > "$out/rtcl.out" 2> "$out/rtcl.err" \
    || { echo "tk: tkprobe failed" >&2; cat "$out/rtcl.err" >&2; exit 2; }

tclsh specs/tk/compare.tcl "$out/tk.out" "$out/rtcl.out" "$scale"
