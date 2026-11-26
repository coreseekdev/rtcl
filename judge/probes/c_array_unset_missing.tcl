# C 阶段 9.1 对齐（2026-11-26）：array unset 的不存在/标量/trace 钉形。
# oracle（tclsh 9.1.0）实测：变量不存在 → 静默成功（8.6 报 can't unset
# "nosuch": no such variable，9.1 改为静默）；标量 → no-op（exists 保持
# 1）；整体 unset 的 unset trace 以 n1=数组名 n2=空 触发一次，trace 抛错
# 不传播（array unset 仍成功、数组移除）。
catch {array unset nosuch} m
puts "A missing: empty=[expr {$m eq {}}]"
set s 5
array unset s
puts "B scalar: exists=[info exists s]"
array set ta {k 1}
array unset ta
puts "C whole: exists=[array exists ta] hask=[info exists ta(k)]"
array set ta {k 1 j 2}
array unset ta k*
puts "D pat: exists=[array exists ta] hask=[info exists ta(k)] hasj=[info exists ta(j)]"
array set tb {k 1}
array unset tb nomatch
puts "E nomatch: exists=[array exists tb] hask=[info exists tb(k)]"
proc tr {n1 n2 op} { puts "TRACE n2empty=[expr {$n2 eq {}}]"; return -code error "trace-fail" }
set tc(k) 1
trace add variable tc unset tr
array unset tc
puts "F trace: exists=[array exists tc]"
