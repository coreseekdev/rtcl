# T7 数组元素读写行为探针（vars.rs set_var/get_var 数组分支快路径改造前钉形）。
# 每行一个断言形，输出与 oracle 逐字节比对；语义敏感区（错误文本、
# errorCode、trace 序、ns/upvar 路由）全覆盖。

proc try {script} {
    if {[catch {uplevel 1 $script} m e] == 0} {
        return "OK:$m"
    }
    return "ERR:$m|code=[dict get [list x [list x $e]] 1 1]?[catch {dict get $e -errorcode} ec]:$ec"
}

# ── 基本读写（顶层 / proc / upvar / 命名空间）──
set a(k) 1
puts "basic=[set a(k)]"
puts "read=$a(k)"
set "a( x )" spaced
puts "space-idx=[array get a { x }],[catch {set a( x )} m]:$m"
set a(b(c)) nested
puts "nested=[set a(b(c))]"
set "a)(p)" parenbase
puts "parenbase=[set a)(p)]"
proc f {} {
    set l(k) in-proc
    set l(j) 2
    return "proc:[set l(k)],[array size l],[array exists l]"
}
puts [f]
proc g {} {
    upvar 0 a(k) x
    set x 5
    return "upvar:$x"
}
puts [g]
puts "after-upvar=[set a(k)],[array size a],[array exists a]"
proc h {} {
    global a
    set a(g) gg
    return "global-link:[array size a]"
}
puts [h]
namespace eval n1 {
    variable t
    set t(1) one
    set t(2) two
}
puts "ns=[array size n1::t],[set n1::t(1)],[array exists n1::t]"
namespace eval n1 { set u(k) uk }
puts "ns-bare=[namespace eval n1 {list [array size u] [set u(k)]}]"

# ── 错误形状（文本与 errorCode 逐字节）──
set scalar 7
puts "read-scalar-elem=[catch {set scalar(k)} m]:$m"
puts "write-scalar-elem=[catch {set scalar(k) 8} m]:$m"
puts "read-missing-elem=[catch {set zz(k)} m]:$m"
puts "read-missing-elem-ec=[catch {set zz(k)} m]:$m:[catch {set ::errorInfo} ei]"
puts "unset-missing-elem=[catch {unset zz(k)} m]:$m"
puts "unset-scalar-elem=[catch {unset scalar(k)} m]:$m"
puts "read-array-base=[catch {set a} m]:$m"
puts "write-array-base=[catch {set a 9} m]:$m"
puts "unset-elem-twice=[set a(del) 1; unset a(del); catch {unset a(del)} m]:$m"

# ── 计数相关语义（P5 后 array size/exists 的答案钉形）──
# （注：`array unset` 对不存在的数组 rtcl 报 no such variable、tclsh
# 静默——预存分歧，未在钉内；数组存在时两种 array unset 都钉。）
set cnt(1) x
set cnt(2) y
set cnt(3) z
puts "cnt3=[array size cnt],[array exists cnt]"
unset cnt(2)
puts "cnt2=[array size cnt],[array exists cnt]"
unset cnt(1)
unset cnt(3)
puts "cnt0=[array size cnt],[array exists cnt]"
set cnt(4) w
puts "cnt1=[array size cnt],[array exists cnt]"
array unset cnt
puts "cnt-unset=[array size cnt],[array exists cnt],[catch {set cnt(4) q}]:"
puts "cnt-recreate=[array size cnt],[array exists cnt]"
array set cnt {}
puts "cnt-empty=[array size cnt],[array exists cnt]"
array set cnt {1 a 2 b}
puts "cnt-set=[array size cnt]"
array set cnt {1 A 2 B 3 C}
puts "cnt-extend=[array size cnt],[set cnt(1)]"
array unset cnt 2
puts "cnt-pat=[array size cnt],[array exists cnt]"

# proc 局部数组的计数与存在性
proc p5 {} {
    array set loc {1 a 2 b}
    unset loc(1)
    return "p5=[array size loc],[array exists loc]"
}
puts [p5]

# upvar 元素链接建容器
proc p6 {} {
    upvar 0 linkt(e) y
    return "p6a=[array exists linkt]"
}
puts [p6]
puts "p6b=[array exists linkt],[array size linkt]"
set linkt(e) via-link
puts "p6c=[array size linkt],[set linkt(e)]"

# ns 数组删除（计数清理路径）
namespace eval n2 { array set dead {1 x 2 y} }
namespace delete n2
puts "ns-del=[catch {array size n2::dead} m]:$m,[array exists n2::dead]"
namespace eval n2 { array set dead {9 z} }
puts "ns-recreate=[array size n2::dead],[set n2::dead(9)]"

# ── trace 交互（元素写/读的触发序）。注：元素 unset trace 的 name1
# 形（tclsh 传全名 ta(k)）与写 trace 改写重入的 errorInfo 帧是 rtcl
# 预存分歧，不在钉内（trace 机制本改动不动）。
set log {}
proc setlog {n v op} { lappend ::log "$op:$n:$v" }
proc rlog {n e op} { lappend ::log "R:$n:$e:$op"; return }
trace add variable ta write setlog
trace add variable ta(k) read rlog
set ta(k) t1
puts "trace-w=$::log"
set ::log {}
puts "trace-r=[set ta(k)]:$::log"
trace remove variable ta write setlog
trace remove variable ta(k) read rlog

# trace 报错传播（元素写报错文本）
proc boom {n e op} { error trace-failed }
trace add variable tc(k) write boom
puts "trace-err=[catch {set tc(k) v} m]:$m"
trace remove variable tc(k) write boom

# ── search 失效（元素写 bump stamp）──
array set s1 {1 a 2 b}
set sid [array startsearch s1]
set first [array nextelement s1 $sid]
set s1(3) c
puts "search-inv=[catch {array donesearch s1 $sid} m]:$m,[catch {array anymore s1 $sid} m2]:$m2"

# ── 整组其它读径的一致性 ──
puts "info-exists=[info exists a(k)],[info exists a(no)]"
puts "lappend=[lappend a(lap) x],[lappend a(lap) y]"
puts "append=[append a(ap) x],[append a(ap) y]"
puts "incr=[incr a(inc)],[incr a(inc) 3]"
unset a(lap) a(ap) a(inc)
puts "unset-partial=[array size a]"
puts "done=t7-arr-elem"
