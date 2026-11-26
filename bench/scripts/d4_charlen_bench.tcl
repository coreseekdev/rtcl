#!/usr/bin/env tclsh
# D4 字符长度缓存基准（Value 惰性 char_len）。
#
# 环境变量 D4_CASE 选案例，clock microseconds 计整段时间，打印总微秒：
#   len-uni      : 24KB 两字节文本 string length × REP（前：每次全串重数；
#                  后：缓存命中 O(1)）
#   len-ascii    : 24KB ASCII string length × REP（对照——不得回退）
#   idx-end-uni  : string index $s end × REP（end 口径要字符长度）
#   idx-mid-uni  : string index $s 6000 × REP（定位走字符步进）
#   rng-uni      : string range $s 10 end-10 × REP（区间两端都要字符口径）
#   first-hit-uni: string first 命中 × REP
#   last-hit-uni : string last 命中 × REP（T7 后 11.16x 的余量项，D4 验收）
#   chain-append : append 16B + string length 交替 × N（增量维护成本可见）
#   chain-map    : string map 重写 + length × N（置无效+重算成本可见）
#   chain-trim   : string trim 重写 + length × N
#   chain-lower  : string tolower 重写 + length × N
# 结果行打印 stderr（防死代码消除），stdout 只出总微秒。

set case $::env(D4_CASE)
set REP 2000
if {[info exists ::env(D4_REP)]} { set REP $::env(D4_REP) }
set N 1000
if {[info exists ::env(D4_N)]} { set N $::env(D4_N) }

proc timed {reps body} {
    set t0 [clock microseconds]
    for {set i 0} {$i < $reps} {incr i} {
        uplevel 1 $body
    }
    return [expr {[clock microseconds] - $t0}]
}

set body ""
switch -- $case {
    len-uni {
        # 两字节 é ×12000 = 24000B/12000 字符。
        set s [string repeat "é" 12000]
        set body {string length $::s}
    }
    len-ascii {
        set s [string repeat "ab" 12000]
        set body {string length $::s}
    }
    idx-end-uni {
        set s [string repeat "é" 12000]
        set body {string index $::s end}
    }
    idx-mid-uni {
        set s [string repeat "é" 12000]
        set body {string index $::s 6000}
    }
    rng-uni {
        set s [string repeat "é" 12000]
        set body {string range $::s 10 end-10}
    }
    first-hit-uni {
        # é ×11990 + éX：命中在串尾（字符位 11990）。
        set s [string repeat "é" 11990]
        append s "éX"
        set body {string first "éX" $::s}
    }
    last-hit-uni {
        # T7 同形：命中字节位 23980，回数的字符口径。
        set s [string repeat "é" 11990]
        append s "éX"
        set body {string last "éX" $::s}
    }
    chain-append {
        # 混合改写链：append 16B（é×8）后立刻问长度——增量维护形。
        set s ""
        set body {
            append ::s "éééééééé"
            string length $::s
        }
        set REP $N
    }
    chain-map {
        # 重写形：每次产新值，缓存必置无效——成本 = 重算一次（2KB 串）。
        set s [string repeat "aé中" 234]
        set body {
            set ::s [string map {a X} $::s]
            string length $::s
        }
        set REP $N
    }
    chain-trim {
        set s [string repeat " é中" 234]
        set body {
            set ::s [string trim $::s]
            string length $::s
        }
        set REP $N
    }
    chain-lower {
        set s [string repeat "É中" 234]
        set body {
            set ::s [string tolower $::s]
            string length $::s
        }
        set REP $N
    }
    default {
        puts stderr "unknown case $case"
        exit 2
    }
}

# 一次热身 + 正式计时。
set x [eval $body]
set us [timed $REP $body]
puts stderr "sink=[string range $x 0 12] len=[string length $::s] case=$case"
puts $us
