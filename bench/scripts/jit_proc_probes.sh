#!/usr/bin/env bash
# M2 seam 差分电池：proc body JIT 执行 seam 的三方逐字节对照。
#   tclsh（语义最高裁判）/ rtcl（无钩子 interp = 零变化基线）/ jitsh（钩子 JIT）
# 用法: bench/scripts/jit_proc_probes.sh   （退出码非 0 = 有分歧）
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl
JIT=./target/release/jitsh
DIR=bench/scripts/jit_proc_probes

pass=0; fail=0; failed=()
for f in "$DIR"/*.tcl; do
    name=$(basename "$f" .tcl)
    o=$(timeout 15 tclsh "$f" 2>&1); oc=$?
    r=$(timeout 15 "$RTCL" -f "$f" 2>&1); rc2=$?
    j=$(timeout 15 "$JIT" -f "$f" 2>&1); jc=$?
    if [ "$o" = "$r" ] && [ "$r" = "$j" ] && [ "$oc" = "$rc2" ] && [ "$rc2" = "$jc" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$name")
        echo "=== FAIL: $name ==="
        diff <(printf '%s' "$r") <(printf '%s' "$j") | head -8
    fi
done
echo "jit_proc_probes: pass=$pass fail=$fail"
[ ${#failed[@]} -gt 0 ] && echo "failed: ${failed[*]}"
[ "$fail" = 0 ]
