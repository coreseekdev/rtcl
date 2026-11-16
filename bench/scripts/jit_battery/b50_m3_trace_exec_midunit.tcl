# M3 失效矩阵⑤：执行 trace 单元中途注册到折叠名（incr）。trace 只在
# 分派/树走查面触发；`trace add execution` 的 epoch bump 让
# exec_bytecode 的逐 op 检查从 add 起重分派（bytecode_applicable 活检
# 压到树走查），JIT classic 面在同一比对点 STALE 重分派——add 之后
# 的每次 incr 都必须触发回调（h2b/h5 探针定谳前：interp 字节码漏
# 触发、JIT incr 靠缓存失效的副作用侥幸、set 宿主臂实漏）。
set n 0
proc tr {args} {puts FIRED}
trace add execution incr {enter} tr
incr n
incr n
puts "n=$n"
