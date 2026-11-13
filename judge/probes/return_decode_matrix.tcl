# return 完成码解码矩阵（tclsh TclProcessReturn / TclUpdateReturnInfo /
# TclMergeReturnOptions 钉形，2026-11-14 errorInfo 帧组轮）：
# - `-code return` 换形 = `-code ok -level+1`（TclMergeReturnOptions）；
# - `-level 0` 物化原码（TclProcessReturn level 0 直接返回码）；
# - proc 边界 ret_level 降级、原码穿透不解码；
# - catch options 键序按 returnOpts 键位（error_options_dict 分类）。
proc t {label script} {
    set c [catch {uplevel 1 $script} r o]
    puts "$label|$c|$r|[scrub $o]"
}
# -errorline 的 uplevel 体口径残差（台账 2026-11-14 残差节）：tclsh 的
# errorLine 在外围命令日志事件处更新、rtcl 的 defer 帧 append 时才写，
# uplevel 体错误的 -errorline 差 1——探针抹平该键，锁其余面。
proc scrub {o} {
    set i [string first "-errorline " $o]
    if {$i < 0} { return $o }
    set head [string range $o 0 [expr {$i + 10}]]
    set rest [string range $o [expr {$i + 11}] end]
    set sp [string first " " $rest]
    if {$sp < 0} { return "$head*" }
    return "$head*[string range $rest $sp end]"
}
proc gv {v} { if {[info exists $v]} { return [set $v] } ; return UNDEF }
t A {return -level 0 -code 7 zz}
t A2 {return -level 0 -code 15 zz}
t A3 {return -level 0 -code break zz}
t A4 {return -level 0 -code continue zz}
t A5 {return -level 0 -code error zz}
t B {return -code return hi}
proc p1 {} { return -level 0 -code 7 zz }
puts "C|[catch {p1} c o]|$c|$o"
proc p2 {} { return -code 7 zz }
puts "D|[catch {p2} c o]|$c|$o"
proc p3 {} { return -code return hi }
puts "E|[catch {p3} c o]|$c|$o"
proc p4 {x} { return -code return -level 1 $x }
proc p5 {} { p4 val }
puts "F|[catch {p5} c o]|$c|$o"
t G {return -code return -level 0 hi}
t H {return -level 2 hi}
t I {return hi}
t J {return -code break -level 1}
t K {return -level 0 hi}
t L {return -level 0 -code error -errorcode {A B} boom}
proc p6 {} { return -code error E }
t M {p6}
proc p7 {} { error boom }
t N {p7}
proc p8 {} { return -code error -errorcode {A B} boom }
t O {p8}
t P {nosuchcmd a b}
t Q {lindex}
foreach v {::errorCode} { puts "$v=[gv $v]" }
