# 文件执行语境（TclEvalEx 逐命令）的 if/while/for 错误帧矩阵（tclsh
# TclNREvalFile -> TclEvalEx -> TEOV 钉形，2026-11-14 errorInfo 帧组轮）：
# 文件级字面体错误挂体出口帧 + 命令 invoker 帧；嵌套体内的构造属其
# 编译单元（无帧）；条件括号错误走 defer；proc 体 = 编译语境无帧。
# 每段以 report 收尾（顶层错误终止文件，故逐段独立探针见
# specs/scripts/file_ctx_*.tcl；本文件锁 catch 内可观察面）。
set ::errorInfo {}
catch {if {1} {error boom}} m
puts "A:$::errorInfo"
set ::errorInfo {}
catch {while {1} {error boom}} m
puts "B:$::errorInfo"
set ::errorInfo {}
catch {for {set i 0} {$i < 1} {incr i} {error boom}} m
puts "C:$::errorInfo"
set ::errorInfo {}
catch {for {set i 0} {$i < 1} {nosuch} {set x 1}} m
puts "D:$::errorInfo"
set ::errorInfo {}
catch {while {[nosuch]} {set x 1}} m
puts "E:$::errorInfo"
set ::errorInfo {}
catch {if {[nosuch]} {set x 1}} m
puts "F:$::errorInfo"
set ::errorInfo {}
catch {eval {if {1} {error boom}}} m
puts "G:$::errorInfo"
set ::errorInfo {}
catch {eval {while {1} {error boom}}} m
puts "H:$::errorInfo"
set ::errorInfo {}
catch {namespace eval ne {if {1} {error boom}}} m
puts "I:$::errorInfo"
set ::errorInfo {}
proc w {} { if {1} {error boom} }
catch {w} m
puts "J:$::errorInfo"
set ::errorInfo {}
proc w2 {} { while {1} {error boom} }
catch {w2} m
puts "K:$::errorInfo"
set ::errorInfo {}
proc w3 {} { for {set i 0} {$i < 1} {incr i} {error boom} }
catch {w3} m
puts "L:$::errorInfo"
set ::errorInfo {}
catch {set x [if {1} {error boom}]} m
puts "M:$::errorInfo"
set ::errorInfo {}
catch {set x [while {1} {error boom}]} m
puts "N:$::errorInfo"
set ::errorInfo {}
if {1} {
    while {1} {error boom}
}
