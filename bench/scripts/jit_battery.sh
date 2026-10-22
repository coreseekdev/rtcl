#!/usr/bin/env bash
# JIT 差分电池（固化的历史探针）：tclsh / rtcl(interp) / jitsh 三方逐字节比。
# 退出码非 0 = 有分歧。单例过滤：jit_battery.sh <name-substr>
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl
JIT=./target/release/jitsh
DIR=bench/scripts/jit_battery
FILTER=${1:-}
pass=0; fail=0; failed=()
for f in "$DIR"/*.tcl; do
    name=$(basename "$f" .tcl)
    [ -n "$FILTER" ] && [[ "$name" != *"$FILTER"* ]] && continue
    o=$(timeout 15 tclsh "$f" 2>&1); oc=$?
    r=$(timeout 15 "$RTCL" -f "$f" 2>&1); rc2=$?
    j=$(timeout 15 "$JIT" -f "$f" 2>&1); jc=$?
    # 首行 `# jit-battery: preexisting` = rtcl 与 tclsh 的已记录分歧——
    # 该例只强制 jit==rtcl（预存面归 judge/specs 管，考卷不改）。
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
        diff <(printf '%s' "$r") <(printf '%s' "$j") | head -8
    fi
done
echo "jit_battery: pass=$pass fail=$fail"
[ ${#failed[@]} -gt 0 ] && echo "failed: ${failed[*]}"
[ "$fail" = 0 ]
