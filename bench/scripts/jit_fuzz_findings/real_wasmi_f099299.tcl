# OPEN: Call-in-Sub 放开后嵌套 proc 定义的错误帧链（case-b 补帧 vs (procedure …) 边界）——
# 嵌套 proc 的 return $undefined_var 错误帧中间层缺失。需专项调查。
proc p2 {n} {
  set k [expr {$s * 2147483648 % $a}]
}
p2 123456789
puts x$ay2
proc p1 {} {
  set m "on"
  if {-(0)} {
  puts "yes"
  proc p3 {} {
  set a $x
  set n $m
}
p3 yes
}
  set i false
}
p1 true
