# 循环条件含 float 比较（Cmp/JumpFalse 全链）
set i 0.0
while {$i < 3.0} {
    puts $i
    set i [expr {$i + 1.25}]
}
puts done
