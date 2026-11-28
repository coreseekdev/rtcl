#!/bin/bash
# D3 F6: body_memo 容量策略——嵌套循环 + 大体不再因挤出回落。
#
# memo 容量 4→32（FIFO 保留）；挤出/失配路径经 eval 的缓存门回字节码
# （F3 重编译 / F5 按请求编译），落树只剩 fallback/trace/手动关断。
#
# 证据（dbg 计数面，rtcl-dbg 构建）：8 个互异大体（各 72KB，总驻集
# ~576KB，远超旧 4 槽）在嵌套循环 + 动态体词里轮转——
#   - unit 计数 > 0（体单元字节码执行）；
#   - tf_eval / tf_big / tf_proc = 0（零回落——挤出后同体照常字节码执行）；
#   - 结果与 --no-bytecode 树遍历逐字节一致。
# memo 容量 >4 的驻集断言在 rtcl-vm lib 测
# test_f6_body_memo_capacity_nested_loops（进程内可读 memo）。
# usage: bash judge/audit-cases/f6-bodymemo.sh [rtcl 路径]
cd "$(dirname "$0")/../.."
RTCL=${1:-target/release/rtcl}
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

cat > "$t/f6.tcl" <<'EOF'
set acc 0
foreach n {1 2 3 4 5 6 7 8} {
    set b$n [string repeat "set w$n 1;" 8000]
}
for {set i 0} {$i < 2} {incr i} {
    foreach m {1 3 5 7} {
        for {set j 0} {$j < 2} {incr j} [set b$m]
    }
    for {set k 0} {$k < 2} {incr k} [set b8]
}
foreach n {1 2 3 4 5 6 7 8} {
    append acc [string length [set b$n]]
}
puts $acc
EOF

"$RTCL" -f "$t/f6.tcl" > "$t/bc.out" 2>&1; bc_rc=$?
"$RTCL" --no-bytecode -f "$t/f6.tcl" > "$t/tree.out" 2>&1; tree_rc=$?
if [ "$bc_rc" != 0 ] || [ "$tree_rc" != 0 ]; then
    echo "F6 FAIL: exit bc=$bc_rc tree=$tree_rc"; head -3 "$t/bc.out" "$t/tree.out"; exit 1
fi
if ! diff -q "$t/bc.out" "$t/tree.out" >/dev/null; then
    echo "F6 FAIL: engines diverge"; diff "$t/bc.out" "$t/tree.out" | head; exit 1
fi
# 8 体 × 72,000 字符（"set w1 1;" × 8000）；acc 以初值 "0" 开头 append。
want="07200072000720007200072000720007200072000"
if [ "$(cat "$t/bc.out")" != "$want" ]; then
    echo "F6 FAIL: wrong result (want $want)"; cat "$t/bc.out"; exit 1
fi

# dbg 计数面（可选——非 dbg 构建无 totals 输出，跳过）。
if RTCL_DBG=1 "$RTCL" -f "$t/f6.tcl" 2>&1 | grep -q "== RTCL_DBG totals"; then
    dbg=$(RTCL_DBG=1 "$RTCL" -f "$t/f6.tcl" 2>&1 | grep -E "^T ")
    echo "$dbg"
    units=$(echo "$dbg" | awk '$2=="unit" {print $3}' | head -1)
    tf=$(echo "$dbg" | awk '$2=="tf_eval" || $2=="tf_big" || $2=="tf_proc" {s+=$3} END{print s+0}')
    if [ -z "$units" ] || [ "$units" -eq 0 ]; then
        echo "F6 FAIL: no bytecode unit execution"; exit 1
    fi
    if [ "${tf:-0}" -ne 0 ]; then
        echo "F6 FAIL: tree-fall events present ($tf)"; exit 1
    fi
    echo "F6 OK: 8x72KB bodies nested-cycling, engines byte-identical, units=$units tf=0"
else
    echo "F6 OK (behavior only; rebuild with --features rtcl-dbg for counters), engines byte-identical"
fi
