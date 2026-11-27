# trace_bench.tcl — 变量 trace 注册/摘除与触发路径基准（D1 步骤 4 验收
# 面：trace 挂 Var 后的注册与触发成本形不回退）。四案例，各自整段
# 墙钟微秒（与 t7_array_bench 同法：stdout 只出总微秒，防死代码消除）：
#   reg-rm    : N 次 trace add + trace remove（同一变量，注册→摘净循环；
#               记录计数归零但宿主占位留存——lingering 键形）
#   fire-w    : 单条 write trace 在 N 次 `set x 1` 上触发（重入守卫 +
#               收集 + eval 求值链）
#   fire-elem : 单条元素 write trace 在 N 次 `set a(k) 1` 上触发
#   notrace-set: 无 trace 基线（N 次裸 set——守卫零成本面）
# 环境变量 TB_N 可调（默认 20000）。

set N 20000
if {[info exists ::env(TB_N)]} { set N $::env(TB_N) }

proc timed {body} {
    set t0 [clock microseconds]
    uplevel 1 $body
    return [expr {[clock microseconds] - $t0}]
}

set ::sink 0

# 1) 注册/摘净循环（同一变量反复 add+remove；记录在 remove 后归零）
set t1 [timed {
    for {set i 0} {$i < $N} {incr i} {
        trace add variable trv write [list incr ::sink]
        trace remove variable trv write [list incr ::sink]
    }
}]

# 2) 整变量 write 触发（触发面：set → 收集 → 回调 eval）
unset -nocomplain fw
trace add variable fw write {incr ::sink;#}
set t2 [timed {
    for {set i 0} {$i < $N} {incr i} {
        set fw $i
    }
}]

# 3) 元素 write 触发
unset -nocomplain fa
set fa(k0) init
trace add variable fa(k) write {incr ::sink;#}
set t3 [timed {
    for {set i 0} {$i < $N} {incr i} {
        set fa(k) $i
    }
}]

# 4) 无 trace 裸写基线（守卫零成本面——n_trace_hosts == 0 快径）
unset -nocomplain nt
set t4 [timed {
    for {set i 0} {$i < $N} {incr i} {
        set nt $i
    }
}]

puts stderr "trace-bench: reg-rm=$t1 fire-w=$t2 fire-elem=$t3 notrace-set=$t4 (N=$N sink=$::sink)"
puts "$t1 $t2 $t3 $t4"
