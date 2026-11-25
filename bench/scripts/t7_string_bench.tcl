#!/usr/bin/env tclsh
# T7 字符串基准（P15 剩余三处，commands/string_cmds.rs 的 string last）。
#
# 环境变量 T7_CASE 选案例，clock microseconds 计整段时间，打印总微秒：
#   last-hit    : 24KB ASCII 文本、模式命中在末尾附近 × REP 次
#   last-miss-sp: 带空格长模式在无空格文本上未命中 × REP 次
#   last-hit-uni: 非 ASCII（两字节）文本、命中位置靠后 × REP 次
# 结果行打印 stderr（防死代码消除），stdout 只出总微秒。

set case $::env(T7_CASE)
set REP 500
if {[info exists ::env(T7_REP)]} { set REP $::env(T7_REP) }

proc timed {reps body} {
    set t0 [clock microseconds]
    for {set i 0} {$i < $reps} {incr i} {
        uplevel 1 $body
    }
    return [expr {[clock microseconds] - $t0}]
}

set text ""
set pat ""
switch -- $case {
    last-hit {
        # 24KB "abab…"，模式在 23961 处（命中）。
        set text [string repeat "ab" 11980]
        append text "abcxyz"
        set pat "abc"
        set body {string last $::pat $::text}
    }
    last-miss-sp {
        # 带空格的长模式、纯无空格文本（未命中，tclsh 靠空格早退）。
        set text [string repeat "a" 24000]
        set pat "the quick brown fox jumps over the lazy dog"
        set body {string last $::pat $::text}
    }
    last-hit-uni {
        # 两字节 é ×11990 + éX：命中字节位 23980、前面 11990 个字符
        # 要数（tclsh 有长度缓存，rtcl 每次重数）。
        set text [string repeat "\u00e9" 11990]
        append text "\u00e9X"
        set pat "\u00e9X"
        set body {string last $::pat $::text}
    }
    default {
        puts stderr "unknown case $case"
        exit 2
    }
}

# 一次热身 + 正式计时。
set x [eval $body]
set us [timed $REP $body]
puts stderr "sink=$x len=[string length $text]"
puts $us
