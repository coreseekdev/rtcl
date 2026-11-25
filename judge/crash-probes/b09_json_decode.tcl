# crash-probe: b09 json decode
# 9.1 注意：json 是 rtcl 的 36 项扩展之一，tclsh（任一版本）无此命令，
# 无 oracle 可钉——期望值来自 rtcl 行为钉形，防回归用（迁移无关）。
source [file join [file dirname [info script]] _chk.tcl]
# Nesting is data: 30000 levels used to run the native stack out; now it
# is a clean error past the nesting limit, and documents within it decode.
set s [string repeat {[} 30000]
append s [string repeat {]} 30000]
set r [catch {json decode $s} e]
chk deep-rc $r 1
set s [string repeat {[} 1000]
append s [string repeat {]} 1000]
chk limit-rc [catch {json decode $s} e] 0
set v [lindex $s 0]
chk deep-schema [catch {json encode x [string repeat {list } 30000]} e] 1
