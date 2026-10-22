# OPEN: Call-in-Sub 放开后嵌套 proc 定义的错误帧链（case-b 补帧 vs (procedure …) 边界）——
# 嵌套 proc 的 return $undefined_var 错误帧中间层缺失。需专项调查。
set s 38
proc p2 {x a} {
  incr s 6
  set a true
  proc p1 {x a} {
  puts "abc"
  return $b
}
p1 1 4611686018427387903
  return $x
}
p2 30 -2147483648
foreach x {-4 2 -5 -2 -1} {
  incr b -5
}
for {set s 0} {$s < 7} {incr s 2} {
  puts "yes"
}
nosuchcmd2 "0x10"
puts "$s"
