#!/usr/bin/env tclsh
# T7 数组基准（HANDOFF §4 T7：P5 + 数组元素读写）。
#
# 环境变量 T7_CASE 选案例，脚本自己用 clock microseconds 计整段时间
# （rtcl 的 time 只报整数微秒，亚微秒一律整段墙钟），打印该案例的
# 总微秒数。案例（REP/KEYS 可被环境变量覆盖）：
#   size-80k     : 8 万元素数组上 `array size` × REP 次
#   exists-80k   : 8 万元素数组上 `array exists`（命中）× REP 次
#   exists-miss  : 8 万元素数组在场时 `array exists zz`（未命中）× REP 次
#   read-50k     : 1 万个既有元素各读 5 轮 = 5 万次元素读
#   write-new    : 5 万次新元素写（数组增长）
#   write-old    : 5 万次同键覆盖写
# 结果行打印 stderr（防死代码消除），stdout 只出总微秒。

set case $::env(T7_CASE)
set REP 300
if {[info exists ::env(T7_REP)]} { set REP $::env(T7_REP) }

proc setup_big {n} {
    for {set i 0} {$i < $n} {incr i} {
        set a(k$i) v$i
    }
    return [array get a]
}

proc timed {reps body} {
    set t0 [clock microseconds]
    for {set i 0} {$i < $reps} {incr i} {
        uplevel 1 $body
    }
    return [expr {[clock microseconds] - $t0}]
}

set us 0
switch -- $case {
    size-80k - exists-80k - exists-miss {
        array set a [setup_big 80000]
        switch -- $case {
            size-80k   { set body {set x [array size a]} }
            exists-80k { set body {set x [array exists a]} }
            exists-miss { set body {set x [array exists zz]} }
        }
        set us [timed $REP $body]
        puts stderr "sink=$x"
    }
    read-50k {
        for {set i 0} {$i < 10000} {incr i} { set a(k$i) v$i }
        set t0 [clock microseconds]
        for {set r 0} {$r < 5} {incr r} {
            for {set i 0} {$i < 10000} {incr i} {
                set x $a(k$i)
            }
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=$x"
    }
    write-new {
        set t0 [clock microseconds]
        for {set i 0} {$i < 50000} {incr i} {
            set b(w$i) $i
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=[array size b]"
    }
    write-old {
        set t0 [clock microseconds]
        for {set i 0} {$i < 50000} {incr i} {
            set b(x) $i
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=$b(x)"
    }
    micro-write {
        # 热键单元素写（time 报不了亚微秒，用整段墙钟）。
        set b(k) 1
        set t0 [clock microseconds]
        for {set i 0} {$i < 100000} {incr i} {
            set b(k) 2
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=$b(k)"
    }
    micro-read {
        set b(k) 1
        set t0 [clock microseconds]
        for {set i 0} {$i < 100000} {incr i} {
            set x $b(k)
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=$x"
    }
    micro-incr {
        set b(k) 0
        set t0 [clock microseconds]
        for {set i 0} {$i < 100000} {incr i} {
            incr b(k)
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=$b(k)"
    }
    micro-append {
        set b(k) ""
        set t0 [clock microseconds]
        for {set i 0} {$i < 100000} {incr i} {
            append b(k) x
        }
        set us [expr {[clock microseconds] - $t0}]
        puts stderr "sink=[string length $b(k)]"
    }
    default {
        puts stderr "unknown case $case"
        exit 2
    }
}
puts $us
