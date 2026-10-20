# OPEN（2026-10-20）：Call-in-Sub 放开实验——lmap wasm 内联 collect 丢失
# （b17 输出 "6" vs "2 4 6"）+ judge 5 文件回归；call_argv 直连消 M1' 的
# double-append 技术成立，lmap 内联需专项调查。另：SEGV 根因（写丢失
# 精确定位）仍 OPEN。
# 发现：SEGV（wasmi+wasmtime 双引擎，~2.2s 崩溃；rtcl/tclsh 为语义正确的死循环 rc=124）。
# 形状：while 死循环体内 lmap（经 call import 站点 eval 路由）+ 字符串操作数位运算 ~($b)。
# 定性：疑似预存（2026-10-19 的 case-b/exit 改动对此形状惰性——无 proc/return/exit/jit_err 路径）；需 gdb 栈定位。
# 状态：已修复（防线）——value_of 对 UNSET（bits 0）输入按空值兜底，
# jit 现进入语义正确的死循环（与 rtcl/tclsh 一致 rc=124）。根因
# （bitnot 慢路径拿到未初始化寄存器 a=0——写丢失的精确定位）仍 OPEN，
# 需带符号 gdb 会话（debug core 无符号——apport 剥离，需直接 gdb run）。
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
