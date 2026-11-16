# M3 失效矩阵①：Tier1 命令单元中途重定义（proc set）。折叠 op 的
# 有效性由 epoch 守卫——重定义经 note_tier1_mutation bump，字节码
# VM 的逐 op 检查（vm_exec ④）与 JIT classic 面的回边比对
# （host.rs ④ 中途 bump）同点重分派：add 之后的 set 走树走查分派
# 遮蔽体（x 未被写入），与 tclsh 的逐命令重解析一致。
set x 5
proc set {args} {return 99}
set x 6
puts $x
