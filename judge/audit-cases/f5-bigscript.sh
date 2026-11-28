#!/bin/bash
# D3 F5: 大脚本直接编译入口——超 PARSE_CACHE_MAX_SCRIPT（默认 1MiB）
# 的顶层 eval 按请求编译执行，不再落树；按请求单元不进任何缓存。
#
# 证据：
#   a) big-unit.sh 族放大到 >1MiB 的 errorInfo 与小单元**逐字节一致**
#      （归一化文件路径与行号后 diff；tclsh 9.1 同形——C55 的树形多一
#      层 `invoked from within "set x …"` 帧不再出现）；
#   b) RTCL_DBG 计数（rtcl-dbg 构建）：unit > 0（字节码单元执行）、
#      tf_eval/tf_big = 0（零回落）。
# usage: bash judge/audit-cases/f5-bigscript.sh [rtcl 路径]
cd "$(dirname "$0")/../.."
RTCL=${1:-target/release/rtcl}
ORACLE=.refer/tcl910/inst/bin/tclsh9.1
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

gen() { # gen <loops> <out>
    { echo "set i 0"
      for ((k = 1; k < $1; k++)); do echo 'for {set j 0} {$j < 1} {incr j} { set k $j }'; done
      echo 'for {set j 0} {$j < 1} {incr j} { set x [expr {1/$nosuchvar}] }'
    } > "$2"
}
gen 100 "$t/small.tcl"
gen 24000 "$t/big.tcl"
size=$(wc -c < "$t/big.tcl")
if [ "$size" -le 1048576 ]; then echo "F5 FAIL: big unit only $size bytes"; exit 1; fi

norm() { sed -e "s|$t/[a-z]*\.tcl|UNIT|g" -e 's/line [0-9][0-9]*/line N/'; }

# rtcl 两臂（-f = 文件执行语境，与本问同路径）。
"$RTCL" -f "$t/big.tcl" 2>&1 | norm > "$t/rtcl_big.norm"
"$RTCL" -f "$t/small.tcl" 2>&1 | norm > "$t/rtcl_small.norm"
# tclsh 两臂（真 tclsh，无 -f）。
"$ORACLE" "$t/big.tcl" 2>&1 | norm > "$t/tcl_big.norm"
"$ORACLE" "$t/small.tcl" 2>&1 | norm > "$t/tcl_small.norm"

if ! diff -q "$t/rtcl_big.norm" "$t/rtcl_small.norm" >/dev/null; then
    echo "F5 FAIL: big-unit errorInfo differs from small unit (tree form extra frame?)"
    diff "$t/rtcl_big.norm" "$t/rtcl_small.norm" | head -8; exit 1
fi
if ! diff -q "$t/rtcl_big.norm" "$t/tcl_big.norm" >/dev/null; then
    echo "F5 FAIL: big-unit errorInfo differs from tclsh 9.1"
    diff "$t/rtcl_big.norm" "$t/tcl_big.norm" | head -8; exit 1
fi

# dbg 计数面（可选——非 dbg 构建无 totals 输出，跳过）。
if RTCL_DBG=1 "$RTCL" -f "$t/big.tcl" 2>&1 | grep -q "== RTCL_DBG totals"; then
    dbg=$(RTCL_DBG=1 "$RTCL" -f "$t/big.tcl" 2>&1 | grep -E "^T ")
    echo "$dbg"
    units=$(echo "$dbg" | awk '$2=="unit" {print $3}' | head -1)
    tf=$(echo "$dbg" | awk '$2=="tf_eval" || $2=="tf_big" {s+=$3} END{print s+0}')
    if [ -z "$units" ] || [ "$units" -eq 0 ]; then
        echo "F5 FAIL: no bytecode unit execution"; exit 1
    fi
    if [ "${tf:-0}" -ne 0 ]; then
        echo "F5 FAIL: tree-fall events present ($tf)"; exit 1
    fi
    echo "F5 OK: size=$size bytes, errorInfo byte-identical (small & tclsh), units=$units tf=0"
else
    echo "F5 OK (behavior only; rebuild with --features rtcl-dbg for counters), errorInfo byte-identical"
fi
