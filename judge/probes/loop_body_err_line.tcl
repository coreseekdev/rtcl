# 体帧行号 provenance（tclsh TclNRWhileObjCmd/ForIterData 钉形，2026-11-17
# fuzz f000054 族收口）：文件级循环体的体出口帧 ("while" body line N) 读
# errorLine——tclsh 的体是编译单元、词括号内联进同单元，最内层日志 = 体
# 相对行；rtcl 的词括号是新单元（括号相对日志），体帧行基改用词括号
# defer 行（属主命令绝对行）还原。catch 体非文件语境：无体帧、
# -errorline = 1（内层括号单元相对行）——前段钉形，尾段未捕获钉形。
# （多行 catch 体括号失败的 -errorline 属 #15 同族 -errorline 残差，
# 另账记录，不在本探针。）
catch {while {1} {set m [expr {~($x)}]}} m o
puts "A:EL=[dict get $o -errorline]"
catch {while {1} {error boom}} m o
puts "B:EL=[dict get $o -errorline]"
set k 0
while {$k < 6} {
  incr k
  set m [expr {~($x)}]
}
