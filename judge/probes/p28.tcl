# 空转（T11b 普查 2026-11-26）：两边死在行8：`dict with d x* …` 的 key 模式要求键已存在（"x*" not known in dictionary），行9–12 未跑
set d {a 1 b 2}
set r [dict with d {set a $b}]
puts "$r | $d"
set d {a 1 b 2}
dict with d {unset b; dict set d c 3}
puts $d
set d {a 1 b 2}
dict with d x* {puts "inner:$a"}
puts [info exists a]
set d {p {q 1} r 5}
dict with d p {puts "nested:$q"}
puts [catch {dict with nodict {zz}} m]; puts $m
