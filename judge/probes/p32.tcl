# 空转（T11b 普查 2026-11-26）：两边死在行2：dict filter script 体 `{list $k $v}` 返回两词（expected boolean value but got a list），行3–12 未跑
puts [dict filter {a 1 b 2 c 3} script {k v} {expr {$v > 1 ? 1 : 0}}]
puts [dict filter {a 1 b 2} script {k v} {list $k $v}]
puts [dict filter {a 1 b 2} script {k v} {}]
set out {}
dict filter {a 1 b 2 c 3} script {k v} {lappend out $k; if {$k eq "b"} {break}; expr 1}
puts $out
set out {}
dict map {k v} {a 1 b 2 c 3} {lappend out $k; if {$k eq "b"} {continue}; string cat $v}
puts "$out|[dict map {k v} {a 1 b 2 c 3} {if {$k eq "b"} {continue}; string cat $v}]"
puts [catch {dict filter {a b} JUNK} m]; puts $m
puts [catch {dict filter {a b c} key} m]; puts $m
puts [catch {dict map "\{x" x x} m]; puts $m
