#!/bin/bash
# D3 F3: epoch 失配重编译不再回落树——Tier1 折叠名被 rename/重定义
# （tier1_epoch bump）后，失配的 proc 体/eval 单元在消费门重编译
# （同一源文本 + 当前遮蔽面，epoch 对齐当前代）并走字节码，不再整单
# 元落树。
#
# 证据（需 rtcl-dbg 特性构建：cargo build --release -p rtcl-cli
# --features rtcl-dbg；env RTCL_DBG 开计数）：
#   - recompile_body / recompile_eval 计数 > 0（每失配体每世代一次）；
#   - unit 计数 > 0（exec_bytecode 单元进入——重编译产物真跑了）；
#   - tf_proc / tf_eval 计数为 0（无回落）；
#   - 行为输出与 --no-bytecode 树遍历逐字节一致（两引擎同形）。
# 无 dbg 构建下本脚本仍验证行为面（输出一致 + 语义形状）。
# usage: bash judge/audit-cases/f3-epoch.sh [rtcl 路径]
cd "$(dirname "$0")/../.."
RTCL=${1:-target/release/rtcl}
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

# 失配面三条腿：
#   1) proc 体：定义后 rename away + 重定义同叶名（两次 bump，两代重编译）
#   2) eval 单元：同文本 bump 前后各 eval 一次（缓存条目失配 → 重编译）
#   3) 语义红线：缺失叶名必须走分派报 invalid command（折叠不得静默
#      执行内建——F3 配套的 rename-away 遮蔽探测）
cat > "$t/f3.tcl" <<'EOF'
proc use_incr {} { incr z 5; return $z }
set z 0
set q 0
eval {incr q 1}
rename incr gone
puts [catch {use_incr} m]
puts $m
puts [catch {eval {incr q 1}} m2]
puts $m2
rename gone incr
puts [use_incr]
puts [eval {incr q 1}]
puts [set q]
EOF

"$RTCL" -f "$t/f3.tcl" > "$t/bc.out" 2>&1; bc_rc=$?
"$RTCL" --no-bytecode -f "$t/f3.tcl" > "$t/tree.out" 2>&1; tree_rc=$?

if [ "$bc_rc" != 0 ] || [ "$tree_rc" != 0 ]; then
    echo "F3 FAIL: exit bc=$bc_rc tree=$tree_rc"; exit 1
fi
if ! diff -q "$t/bc.out" "$t/tree.out" >/dev/null; then
    echo "F3 FAIL: engines diverge"; diff "$t/bc.out" "$t/tree.out" | head; exit 1
fi
want=$'1\ninvalid command name "incr"\n1\ninvalid command name "incr"\n5\n2\n2'
if [ "$(cat "$t/bc.out")" != "$want" ]; then
    echo "F3 FAIL: wrong semantics"; cat "$t/bc.out"; exit 1
fi

# dbg 计数面（可选——非 dbg 构建无 totals 输出，跳过）。
if RTCL_DBG=1 "$RTCL" -f "$t/f3.tcl" 2>&1 | grep -q "== RTCL_DBG totals"; then
    dbg=$(RTCL_DBG=1 "$RTCL" -f "$t/f3.tcl" 2>&1 | grep -E "^T ")
    echo "$dbg"
    recomp=$(echo "$dbg" | awk '$2 ~ /^recompile_(body|eval)$/ {s+=$3} END{print s+0}')
    units=$(echo "$dbg" | awk '$2=="unit" {print $3}' | head -1)
    tf=$(echo "$dbg" | awk '$2=="tf_proc" || $2=="tf_eval" {s+=$3} END{print s+0}')
    if [ "${recomp:-0}" -lt 4 ]; then
        echo "F3 FAIL: expected >=4 recompiles (2 bodies x 2 generations), got ${recomp:-0}"; exit 1
    fi
    if [ -z "$units" ] || [ "$units" -eq 0 ]; then
        echo "F3 FAIL: no bytecode unit executions"; exit 1
    fi
    if [ "${tf:-0}" -ne 0 ]; then
        echo "F3 FAIL: tree-fall events present ($tf)"; exit 1
    fi
    echo "F3 OK: recompiles=$recomp units=$units tf=0, engines byte-identical"
else
    echo "F3 OK (behavior only; rebuild with --features rtcl-dbg for counters), engines byte-identical"
fi
