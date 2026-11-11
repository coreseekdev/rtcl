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
metrics=$?

timeout 60 wish specs/tk/geometry.tcl > "$out/geo-tk.out" 2> "$out/geo-tk.err" \
    || { echo "tk: wish geometry failed" >&2; cat "$out/geo-tk.err" >&2; exit 2; }
timeout 60 "$PROBE" specs/tk/geometry.tcl > "$out/geo-rtcl.out" 2> "$out/geo-rtcl.err" \
    || { echo "tk: tkprobe geometry failed" >&2; cat "$out/geo-rtcl.err" >&2; exit 2; }
tclsh specs/tk/compare-geo.tcl "$out/geo-tk.out" "$out/geo-rtcl.out" "$scale"
geo=$?

# Third pass: event % substitutions, exact diff.  Two lines legitimately
# diverge: an empty %A (Ctrl key, or Return) inside a list-form binding
# keeps its empty ARGUMENT in Tk (which evaluates list objects per
# element) but collapses in rtcl's textual evaluation — known difference,
# see docs/UI-NATIVE-TK.md.
timeout 60 "$PROBE" specs/tk/events.tcl > "$out/ev-rtcl.out" 2> "$out/ev-rtcl.err" \
    || { echo "tk: tkprobe events failed" >&2; cat "$out/ev-rtcl.err" >&2; exit 2; }
timeout 60 wish specs/tk/events.tcl > "$out/ev-tk.out" 2> "$out/ev-tk.err" \
    || { echo "tk: wish events failed" >&2; cat "$out/ev-tk.err" >&2; exit 2; }
# The two key-a/key-ret lines carry an empty %A, whose empty argument Tk
# preserves (list-element evaluation) and rtcl's textual evaluation
# collapses — the documented known difference, filtered on both sides.
known='EV\\|key-(a|ret) N='
ev_diff=$(diff \
    <(tr -d '\r' < "$out/ev-tk.out" | grep -vE "$known") \
    <(tr -d '\r' < "$out/ev-rtcl.out" | grep -vE "$known") \
    || true)
if [ -n "$ev_diff" ]; then
    printf '%s\n' "$ev_diff"
    echo "tk: events differ"
    geo=1
fi

if [ "$metrics" -ne 0 ] || [ "$geo" -ne 0 ]; then exit 1; fi
