#!/usr/bin/env bash
# 正则引擎语义差分评估（tclsh 8.6.17 vs rtcl）。
#
# 用法（必须经 512M 护栏）：
#   specs/tclmem.sh 'bash specs/regex/run.sh'
#
# 输出每个语料的差异数与分类；评估用，不作为提交门禁（已知差异见
# specs/regex-engine-eval.md）。
set -u
cd "$(dirname "$0")/../.."

RTCL=./target/release/rtcl
[ ! -x "$RTCL" ] && { echo "regex-eval: $RTCL not found; cargo build --release -p rtcl-cli first" >&2; exit 2; }

# Spencer Rust 引擎（feature regexp-spencer）验收：整体 span + 捕获。
if [ -x target/release/examples/spencer_span ]; then
    python3 specs/regex/spencer-overall.py | head -1
fi
if [ -x target/release/examples/spencer_captures ]; then
    python3 specs/regex/spencer-captures.py | head -1
fi

for f in specs/regex/semantics.tcl specs/regex/random-4000.tcl; do
    [ -e "$f" ] || continue
    base=$(basename "$f" .tcl)
    tclsh "$f" > "/tmp/rtcl-regex-$base-o.txt" 2>&1
    "$RTCL" -f "$f" > "/tmp/rtcl-regex-$base-r.txt" 2>&1
    echo "== $base =="
    python3 specs/regex/analyze.py "/tmp/rtcl-regex-$base-o.txt" "/tmp/rtcl-regex-$base-r.txt"
done

# BRE/ERE 方言与 -about re_info 标志（Spencer 默认引擎）。
for f in specs/regex/dialect-probes.tcl specs/regex/about-probes.tcl; do
    base=$(basename "$f" .tcl)
    tclsh "$f" > "/tmp/rtcl-regex-$base-o.txt" 2>&1
    "$RTCL" -f "$f" > "/tmp/rtcl-regex-$base-r.txt" 2>&1
    if diff -q "/tmp/rtcl-regex-$base-o.txt" "/tmp/rtcl-regex-$base-r.txt" >/dev/null; then
        echo "== $base: diff=0 =="
    else
        echo "== $base: DIFF =="
        diff "/tmp/rtcl-regex-$base-o.txt" "/tmp/rtcl-regex-$base-r.txt" | head -20
    fi
done

# regexp/regsub 命令面随机差分（3500 例，含 -start/-all/-indices/
# match vars/regsub 替换串）。
if command -v python3 >/dev/null 2>&1; then
    python3 specs/regex/gen-cmd-fuzz.py > /tmp/rtcl-regex-cmdfuzz.tcl
    tclsh /tmp/rtcl-regex-cmdfuzz.tcl > /tmp/rtcl-regex-cmdfuzz-o.txt 2>&1
    "$RTCL" -f /tmp/rtcl-regex-cmdfuzz.tcl > /tmp/rtcl-regex-cmdfuzz-r.txt 2>&1
    if diff -q /tmp/rtcl-regex-cmdfuzz-o.txt /tmp/rtcl-regex-cmdfuzz-r.txt >/dev/null; then
        echo "== cmd-fuzz: diff=0 =="
    else
        echo "== cmd-fuzz: DIFF =="
        diff /tmp/rtcl-regex-cmdfuzz-o.txt /tmp/rtcl-regex-cmdfuzz-r.txt | head -20
    fi
fi
