# M3 失效矩阵③：ensemble create 的叶名遮蔽 Tier1 折叠名。ensemble
# 注册在 dispatch 解析链 builtins 之前——`incr` 自创建起解析到
# ensemble；折叠 incr 的单元经 bump/逐 op epoch 检查（interp）与
# STALE 重分派（JIT）退回动态解析，报 unknown subcommand（与
# tclsh 的逐命令重解析一致）。
set n 5
namespace eval incr {namespace ensemble create}
incr n
puts "n=$n"
