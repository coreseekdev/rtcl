# rules.tcl —— 规则/告警引擎（docs/ROADMAP.md §3）。规则 = 脚本：
# 条件（-when）与动作（-do）都是 Tcl 片段，可存档、可重放（规则即
# 脚本，审计/复现随脚本走）。after 轮询驱动，**false→true 沿触发**
# 一次（告警去抖），条件回落后重新武装。
#
#   rules::add 名称 -every MS -when {条件} -do {动作}
#   rules::start/stop/remove 名称
#   rules::list              平铺 {名称 active armed fires errors …}
#   rules::fires 名称        累计触发次数
#   rules::errors            累计条件/动作错误次数（全部规则合计）
#
# 条件脚本在全局作用域求值，结果按 Tcl 布尔解释；动作脚本同域执行。
# 错误既不静默也不杀循环：累计计数 + 每个"错误期"只打一条 stderr，
# 恢复（条件为假或动作成功）后重新武装、下次再犯再报。动作失败仍
# 消耗这一次沿（armed 置 0，不重试轰炸）——语义记录在下。
#
# wasm 主线程形态没有自发事件循环：宿主须装心跳把到点的 after 事件
# 泵出来（www/event-loop.js；Worker 形态下是它自己的循环）。
#
# 动作三形态（ROADMAP）：通知 = ui::model set 进 log/toast 位；
# webhook = http::post；新 surface = ui::batch/ui::surface——都是
# 普通 Tcl，引擎不特判。

namespace eval ::rules {
    variable defs [dict create]     ;# name -> dict(every when do active armed fires errors id last_err)
    variable errors 0
}

proc ::rules::add {name args} {
    variable defs
    if {[dict exists $defs $name]} {
        error "rules::add: rule \"$name\" already exists"
    }
    set every 1000
    set when {1}
    set do {}
    foreach {k v} $args {
        switch -- $k {
            -every { set every $v }
            -when  { set when $v }
            -do    { set do $v }
            default { error "rules::add: unknown option \"$k\"" }
        }
    }
    if {![string is integer -strict $every] || $every < 1} {
        error "rules::add: -every must be a positive integer (ms)"
    }
    if {$do eq ""} {
        error "rules::add: -do action script is required"
    }
    dict set defs $name [dict create every $every when $when do $do \
        active 1 armed 1 fires 0 errors 0 last_err 0 id ""]
    ::rules::_schedule $name
    return $name
}

proc ::rules::_schedule {name} {
    variable defs
    set r [dict get $defs $name]
    # 注意 [::list]：::rules::list 是本命名空间的命令，非限定名 list
    # 在这里被遮蔽（Tcl 命令解析先查当前命名空间）。
    dict set r id [after [dict get $r every] [::list ::rules::_tick $name]]
    dict set defs $name $r
}

# 一次评估（after 到点由泵调用；也在 add/start 后串行续期）
proc ::rules::_tick {name} {
    variable defs
    variable errors
    if {![dict exists $defs $name]} { return }   ;# 已删除：不再续期
    set r [dict get $defs $name]
    if {![dict get $r active]} { return }

    set cond 0
    set failed 0
    if {[catch {uplevel #0 [dict get $r when]} cval]} {
        set failed 1
        set cmsg $cval
    } elseif {[catch {expr {$cval ? 1 : 0}} cbool]} {
        set failed 1
        set cmsg $cbool
    } else {
        set cond $cbool
    }
    if {$failed} {
        incr errors
        dict incr r errors
        if {![dict get $r last_err]} {
            puts stderr "rules: condition error in \"$name\": $cmsg"
            dict set r last_err 1
        }
    } else {
        dict set r last_err 0
    }

    if {!$failed && $cond && [dict get $r armed]} {
        # false→true 沿：跑动作（失败计入并同样消耗沿，不重试轰炸）
        if {[catch {uplevel #0 [dict get $r do]} amsg]} {
            incr errors
            dict incr r errors
            if {![dict get $r last_err]} {
                puts stderr "rules: action error in \"$name\": $amsg"
                dict set r last_err 1
            }
        } else {
            dict incr r fires
        }
        dict set r armed 0
    } elseif {!$failed && !$cond} {
        dict set r armed 1
    }

    dict set defs $name $r
    ::rules::_schedule $name
}

proc ::rules::_get {name} {
    variable defs
    if {![dict exists $defs $name]} {
        error "rules: unknown rule \"$name\""
    }
    return [dict get $defs $name]
}

proc ::rules::start {name} {
    variable defs
    set r [::rules::_get $name]
    if {[dict get $r active]} { return }
    dict set r active 1
    dict set defs $name $r
    ::rules::_schedule $name
}

proc ::rules::stop {name} {
    variable defs
    set r [::rules::_get $name]
    if {![dict get $r active]} { return }
    if {[dict get $r id] ne ""} { after cancel [dict get $r id] }
    dict set r active 0
    dict set r id ""
    dict set defs $name $r
}

proc ::rules::remove {name} {
    variable defs
    ::rules::stop $name
    dict unset defs $name
}

proc ::rules::list {} {
    variable defs
    set out [::list]   ;# 注意 [::list]：同名命令在本命名空间遮蔽（见 _schedule 注）
    foreach name [lsort [dict keys $defs]] {
        set r [dict get $defs $name]
        lappend out $name [dict get $r active] [dict get $r armed] \
            [dict get $r fires] [dict get $r errors]
    }
    return $out
}

proc ::rules::fires {name} {
    return [dict get [::rules::_get $name] fires]
}

proc ::rules::errors {} {
    variable errors
    return $errors
}
