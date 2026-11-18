#!/usr/bin/env bash
# C-float（§5）float 语义矩阵三方逐字节差分：
#   tclsh（语义最高裁判）/ rtcl（无钩子 interp）/ jitsh（JIT 引擎）。
# JIT_BIN 覆盖引擎执行体——同一 runner 分别驱动 lane-off（现状回归）与
# lane-on（jit-float-lane 新路径），各轮都须与 tclsh 逐字节一致。
# 用法: bench/scripts/jit_float_probes.sh   （退出码非 0 = 有分歧）
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl
JIT=${JIT_BIN:-./target/release/jitsh}
DIR=bench/scripts/jit_float_probes

pass=0; fail=0; failed=()
for f in "$DIR"/*.tcl; do
    name=$(basename "$f" .tcl)
    o=$(timeout 15 tclsh "$f" 2>&1); oc=$?
    r=$(timeout 15 "$RTCL" -f "$f" 2>&1); rc2=$?
    j=$(timeout 15 "$JIT" -f "$f" 2>&1); jc=$?
    # 首行 `# jit-battery: preexisting` = rtcl(interp) 与 tclsh 的已记录
    # 分歧（如 NaN 乘积产物：tclsh 8.6 对 Inf*0.0 报 domain error，interp
    # 放行 NaN）——该例只强制 jit==rtcl（预存面归 judge/specs 管；
    # JIT 的语义 ground truth = 解释器）。
    if head -1 "$f" | grep -q '^# jit-battery: preexisting'; then
        if [ "$r" = "$j" ] && [ "$rc2" = "$jc" ]; then
            pass=$((pass+1)); continue
        fi
        fail=$((fail+1)); failed+=("$name")
        echo "=== FAIL(jit≠rtcl): $name ==="
        continue
    fi
    if [ "$o" = "$r" ] && [ "$r" = "$j" ] && [ "$oc" = "$rc2" ] && [ "$rc2" = "$jc" ]; then
        pass=$((pass+1))
    else
        fail=$((fail+1)); failed+=("$name")
        echo "=== FAIL: $name ==="
        echo "--- tclsh:"; printf '%s\n' "$o" | head -6
        echo "--- jit:"; printf '%s\n' "$j" | head -6
    fi
done
echo "jit_float_probes: pass=$pass fail=$fail (engine=$JIT)"
[ ${#failed[@]} -gt 0 ] && echo "failed: ${failed[*]}"
[ "$fail" = 0 ]
