# M2 slot 缓存失效矩阵①：命令调用（call import）清缓存窗口——循环内
# 混合命令与算术，命令后变量读必须走宿主（缓存整表失效）。
set s 0
set i 0
while {$i < 5} {
    set s [expr {$s + $i}]
    set len [string length "ab"]
    incr i
}
puts "s=$s i=$i len=$len"
