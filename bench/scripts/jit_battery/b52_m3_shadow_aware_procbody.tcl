# M3 失效矩阵④：遮蔽后定义的 proc 体不得内联折叠遮蔽名。epoch 戳
# 只防「编译后」的遮蔽（bump 退役既有单元）；遮蔽存在时新定义的
# proc 体由 shadowed_tier1_folds 点名 skip_folds——set 走分派活绑定
# （遮蔽体返回 SHADOW，x 未写入），return $x 报 can't read "x"。
# 修复前（h1 探针定谳）：体编译盖章新 epoch 过 Site A 门，折叠
# StoreVar 直写 x——interp 字节码与 JIT proc 钩子双方输出 6。
proc set {args} {return SHADOW}
proc foo {} {set x 6; return $x}
puts [foo]
