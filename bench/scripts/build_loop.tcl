# build_loop.tcl — build-only 基准（for+lappend 循环，fe_bench 的 build 侧隔离）。
proc build {n} {
    set L {}
    for {set i 0} {$i < $n} {incr i} { lappend L $i }
    return $L
}
set z [build 200000]
puts [llength $z]
