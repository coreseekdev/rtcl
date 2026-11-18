# 编译体热点形：proc+for 循环内 float 累加（fast path 实跑）
proc f {} {
    set x 1.5
    set r 0.0
    for {set i 0} {$i < 5} {incr i} {
        set r [expr {$r + $x * $i}]
    }
    return $r
}
puts [f]
proc g {} {
    set acc 1.0
    for {set i 0} {$i < 8} {incr i} {
        set acc [expr {$acc * 1.25 + 0.5}]
    }
    return $acc
}
puts [g]
