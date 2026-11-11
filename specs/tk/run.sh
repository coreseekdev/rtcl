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

# Fourth pass: selection ownership/handlers and the virtual event table,
# exact diff (info outputs are lsort-ed inside the script).
timeout 60 wish specs/tk/selection-virtual.tcl > "$out/sv-tk.out" 2> "$out/sv-tk.err" \
    || { echo "tk: wish selection failed" >&2; cat "$out/sv-tk.err" >&2; exit 2; }
timeout 60 "$PROBE" specs/tk/selection-virtual.tcl > "$out/sv-rtcl.out" 2> "$out/sv-rtcl.err" \
    || { echo "tk: tkprobe selection failed" >&2; cat "$out/sv-rtcl.err" >&2; exit 2; }
sv_diff=$(diff \
    <(tr -d '\r' < "$out/sv-tk.out") \
    <(tr -d '\r' < "$out/sv-rtcl.out") \
    || true)
if [ -n "$sv_diff" ]; then
    printf '%s\n' "$sv_diff"
    echo "tk: selection/virtual differ"
    geo=1
fi

# Fifth pass: the winfo family, exact diff after scale normalization.
# Known environment differences, filtered on both sides: the root
# window's class (Tk derives it from the script's name), the default
# font's family (rtcl's TkTextFont is DejaVu Sans, Tk's is Noto Sans —
# WID/GEOM follow it), the screen's pixel size (Xft.dpi scaling) and
# X's visuals enumeration.
timeout 60 wish specs/tk/winfo.tcl > "$out/wi-tk.out" 2> "$out/wi-tk.err" \
    || { echo "tk: wish winfo failed" >&2; cat "$out/wi-tk.err" >&2; exit 2; }
timeout 60 "$PROBE" specs/tk/winfo.tcl > "$out/wi-rtcl.out" 2> "$out/wi-rtcl.err" \
    || { echo "tk: tkprobe winfo failed" >&2; cat "$out/wi-rtcl.err" >&2; exit 2; }
wi_norm() {
    tr -d '\r' < "$1" | grep -vE '^CLS|^WID|^GEOM|^SCR|^VIS' | sed \
        -e 's/[0-9]\+x[0-9]\++0+0/SIZExSIZE+0+0/' \
        -e 's/bad option "nosuch": must be.*/BAD-OPTION-LIST/'
}
wi_diff=$(diff <(wi_norm "$out/wi-tk.out") <(wi_norm "$out/wi-rtcl.out") || true)
if [ -n "$wi_diff" ]; then
    printf '%s\n' "$wi_diff"
    echo "tk: winfo differs"
    geo=1
fi

if [ "$metrics" -ne 0 ] || [ "$geo" -ne 0 ]; then exit 1; fi
