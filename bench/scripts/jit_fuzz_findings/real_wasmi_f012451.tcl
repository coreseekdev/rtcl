# 发现：SEGV（wasmi+wasmtime 双引擎，~2.2s 崩溃；rtcl/tclsh 为语义正确的死循环 rc=124）。
# 形状：while 死循环体内 lmap（经 call import 站点 eval 路由）+ 字符串操作数位运算 ~($b)。
# 定性：疑似预存（2026-10-19 的 case-b/exit 改动对此形状惰性——无 proc/return/exit/jit_err 路径）；需 gdb 栈定位。
# 状态：OPEN——下一个任务包首要项。
set s -1152921504606846976
puts " 42 "
foreach b {5 -3 3 3 4} {
  puts "foo bar"
  continue
}
set k 0
while {$k < 5} {
  incr k
  lmap i {1 2 3 4} {
  set k [expr {~($b)}]
  expr {$i * 2}
}
  if {$k > 4} break
}
