#!/usr/bin/env bash
# 多行差分脚本跑手：tclsh vs rtcl -f，逐脚本 stdout 比对。
#
# 用法（必须经 512M 护栏，见 specs/tclmem.sh）：
#   specs/tclmem.sh 'bash specs/scripts/run.sh'
#   specs/tclmem.sh 'bash specs/scripts/run.sh translation'
#
# 与 specs/probe.sh（一行一探针）互补：这里是多行脚本（临时文件、
# proc、循环），用于 translation/chan copy/encoding 矩阵这类需要
# 组合状态的差分。脚本必须确定性（无 clock/random）。
set -u
cd "$(dirname "$0")/../.."

RTCL=./target/release/rtcl
ORACLE=tclsh
[ ! -x "$RTCL" ] && { echo "scripts: $RTCL not found; cargo build --release -p rtcl-cli first" >&2; exit 2; }

pass=0; fail=0; failed=()
for f in specs/scripts/${1:-*}.tcl; do
    [ -e "$f" ] || continue
    name=$(basename "$f")
    o_out=$(timeout 60 "$ORACLE" "$f" 2>&1); o_code=$?
    r_out=$(timeout 60 "$RTCL" -f "$f" 2>&1); r_code=$?
    if [ "$o_out" = "$r_out" ] && [ "$o_code" = "$r_code" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$name")
    fi
done

echo "scripts: pass=$pass fail=$fail"
if [ "$fail" -gt 0 ]; then
    printf '  DIFF: %s\n' "${failed[@]}"
    exit 1
fi
