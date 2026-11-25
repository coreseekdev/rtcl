# crash-probe: b10 json encode
# 9.1 注意：json 是 rtcl 的 36 项扩展之一，tclsh（任一版本）无此命令，
# 无 oracle 可钉——期望值来自 rtcl 行为钉形，防回归用（迁移无关）。
source [file join [file dirname [info script]] _chk.tcl]
set l x
for {set j 0} {$j<20000} {incr j} { set l [list $l] }
set r [catch {json encode $l} e]
chk rc $r 0
