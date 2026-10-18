# log.tcl —— 事件存储（内存 ring buffer）与查询管道词表
# （docs/ROADMAP.md §3 起步版：wasm 侧先用内存 ring；native 的
# SQLite/FTS5 与 IndexedDB 是后续形态）。
#
# 事件 = JSON 对象文本（与 ui::model 行、http::jsonl 输出同形——
# 摄取链路 http::jsonl → log::ingest-lines 直通）。查询输出全是
# Tcl list（平铺对/三元组），可直接 ui::model set 进 table/chart：
#
#   log::capacity ?n?            环容量（默认 10000；超出丢最旧）
#   log::ingest 事件 ?-source s? 入环；返回序号
#   log::ingest-lines 文本 ?-source s?   JSON Lines 批入；返回条数
#   log::count ?-source s?
#   log::tail n ?-source s?      最新 n 条（新在尾）
#   log::search 子串 ?-limit n? ?-source s?   子串匹配，新到旧
#   log::stats 字段 ?-limit n?   平铺 {值 次数 …}，次数降序、并列值升序
#   log::top 字段 n              = log::stats -limit n
#   log::hist 字段 桶数          数值直方图，平铺 {start end count …}
#
# 字段取值按 JSON 对象顶层键（缺字段的事件跳过该查询，不报错）。

namespace eval ::log {
    variable events {}
    variable sources {}
    variable seq 0
    variable capacity 10000
}

proc ::log::capacity {args} {
    variable capacity
    if {[llength $args] == 0} { return $capacity }
    lassign $args n
    if {![string is integer -strict $n] || $n < 1} {
        error "log::capacity: must be a positive integer"
    }
    set capacity $n
    ::log::_trim
    return $capacity
}

proc ::log::_trim {} {
    variable events
    variable sources
    variable capacity
    set over [expr {[llength $events] - $capacity}]
    if {$over > 0} {
        set events [lrange $events $over end]
        set sources [lrange $sources $over end]
    }
}

proc ::log::ingest {event args} {
    variable events
    variable sources
    variable seq
    set source ""
    foreach {k v} $args {
        if {$k eq "-source"} { set source $v } else {
            error "log::ingest: unknown option \"$k\""
        }
    }
    if {$event eq "" || [string index $event 0] ne "\{" || [catch {json decode $event}]} {
        error "log::ingest: event must be a JSON object text"
    }
    lappend events $event
    lappend sources $source
    incr seq
    ::log::_trim
    return $seq
}

proc ::log::ingest-lines {text args} {
    variable seq
    set n 0
    foreach line [::log::_lines $text] {
        ::log::ingest $line {*}$args
        incr n
    }
    return $n
}

# JSON Lines 切分（与 http::jsonl 同规则：非空行、trim）
proc ::log::_lines {text} {
    set out [list]
    foreach line [split $text \n] {
        set line [string trim $line]
        if {$line eq ""} { continue }
        lappend out $line
    }
    return $out
}

# 过滤出（下标列表）供各查询共享
proc ::log::_select {source} {
    variable sources
    if {$source eq ""} {
        set n [llength $sources]
        set out [list]
        for {set i 0} {$i < $n} {incr i} { lappend out $i }
        return $out
    }
    set out [list]
    set i 0
    foreach s $sources {
        if {$s eq $source} { lappend out $i }
        incr i
    }
    return $out
}

proc ::log::count {args} {
    variable sources
    set source ""
    foreach {k v} $args {
        if {$k eq "-source"} { set source $v } else {
            error "log::count: unknown option \"$k\""
        }
    }
    if {$source eq ""} { return [llength $sources] }
    set n 0
    foreach s $sources { if {$s eq $source} { incr n } }
    return $n
}

proc ::log::tail {n args} {
    variable events
    if {![string is integer -strict $n] || $n < 0} {
        error "log::tail: n must be a non-negative integer"
    }
    set source ""
    foreach {k v} $args {
        if {$k eq "-source"} { set source $v } else {
            error "log::tail: unknown option \"$k\""
        }
    }
    set out [list]
    set idxs [::log::_select $source]
    set total [llength $idxs]
    for {set j [expr {$total - $n}]} {$j < $total} {incr j} {
        if {$j < 0} { continue }
        lappend out [lindex $events [lindex $idxs $j]]
    }
    return $out
}

proc ::log::search {pattern args} {
    variable events
    set limit 100
    set source ""
    foreach {k v} $args {
        switch -- $k {
            -limit  { set limit $v }
            -source { set source $v }
            default { error "log::search: unknown option \"$k\"" }
        }
    }
    if {[string is integer -strict $limit] == 0 || $limit < 0} {
        error "log::search: -limit must be a non-negative integer"
    }
    set out [list]
    set idxs [::log::_select $source]
    for {set j [expr {[llength $idxs] - 1}]} {$j >= 0} {incr j -1} {
        if {[llength $out] >= $limit} { break }
        set ev [lindex $events [lindex $idxs $j]]
        if {[string first $pattern $ev] >= 0} { lappend out $ev }
    }
    return $out
}

# 计数比较：次数降序，并列按值升序（字符串比较）
proc ::log::_cmp {a b} {
    if {[lindex $a 1] != [lindex $b 1]} {
        expr {[lindex $a 1] > [lindex $b 1] ? -1 : 1}
    } else {
        string compare [lindex $a 0] [lindex $b 0]
    }
}

proc ::log::stats {field args} {
    variable events
    set limit 0
    set source ""
    foreach {k v} $args {
        switch -- $k {
            -limit  { set limit $v }
            -source { set source $v }
            default { error "log::stats: unknown option \"$k\"" }
        }
    }
    set counts [dict create]
    foreach idx [::log::_select $source] {
        set d [json decode [lindex $events $idx]]
        if {![dict exists $d $field]} { continue }
        set v [dict get $d $field]
        if {[dict exists $counts $v]} {
            dict set counts $v [expr {[dict get $counts $v] + 1}]
        } else {
            dict set counts $v 1
        }
    }
    set pairs [list]
    dict for {v c} $counts { lappend pairs [list $v $c] }
    if {[llength $pairs] > 1} {
        set pairs [lsort -command ::log::_cmp $pairs]
    }
    set out [list]
    foreach pair $pairs {
        if {$limit > 0 && [llength $out] / 2 >= $limit} { break }
        lappend out {*}$pair
    }
    return $out
}

proc ::log::top {field n} {
    if {![string is integer -strict $n] || $n < 1} {
        error "log::top: n must be a positive integer"
    }
    return [::log::stats $field -limit $n]
}

proc ::log::hist {field buckets args} {
    variable events
    if {![string is integer -strict $buckets] || $buckets < 1} {
        error "log::hist: buckets must be a positive integer"
    }
    set source ""
    foreach {k v} $args {
        switch -- $k {
            -source { set source $v }
            default  { error "log::hist: unknown option \"$k\"" }
        }
    }
    set vals [list]
    foreach idx [::log::_select $source] {
        set d [json decode [lindex $events $idx]]
        if {![dict exists $d $field]} { continue }
        set v [dict get $d $field]
        # 只收数值（true/false 不是 double；注意不能再用 string is
        # boolean 反筛——它把数值 0/1 也认成布尔）
        if {![string is double -strict $v]} { continue }
        lappend vals $v
    }
    if {[llength $vals] == 0} {
        set zeros [list]
        for {set i 0} {$i < $buckets} {incr i} { lappend zeros 0 0 0 }
        return $zeros
    }
    set lo [lindex [lsort -real $vals] 0]
    set hi [lindex [lsort -real $vals] end]
    set width [expr {$hi == $lo ? 1.0 : ($hi - $lo) / double($buckets)}]
    set counts [list]
    for {set i 0} {$i < $buckets} {incr i} { lappend counts 0 }
    foreach v $vals {
        set b [expr {int(($v - $lo) / $width)}]
        if {$b >= $buckets} { set b [expr {$buckets - 1}] }
        if {$b < 0} { set b 0 }
        lset counts $b [expr {[lindex $counts $b] + 1}]
    }
    set out [list]
    for {set i 0} {$i < $buckets} {incr i} {
        set start [expr {$lo + $i * $width}]
        set end [expr {$lo + ($i + 1) * $width}]
        lappend out [format %.6g $start] [format %.6g $end] [lindex $counts $i]
    }
    return $out
}
