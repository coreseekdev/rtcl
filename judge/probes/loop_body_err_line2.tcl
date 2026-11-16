# 嵌套循环体的体帧行号（fuzz f000213 收口钉形）：for 体嵌在 while 体内
# 属同一编译单元（tclsh 内联，无 for 体帧——p20d），错误传出时只有最
# 外层 while 的体出口帧，行 = 括号属主命令（for 体内 set k）的体相对
# 行 4。尾段未捕获钉形。
catch {while {1} {for {set i 0} {$i < 1} {incr i} {error boom}}} m o
puts "A:EL=[dict get $o -errorline]"
set m 0
while {$m < 10} {
  incr m
  for {set x 0} {$x < 3} {incr x 3} {
    set k [expr {$i}]
  }
}
