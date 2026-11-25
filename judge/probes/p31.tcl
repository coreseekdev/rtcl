# 空转（T11b 普查 2026-11-26）：两边死在末条检查行7：dict filter 的 script 体 `$v > 1 ? …` 按 Tcl 脚本求值（缺 expr{}）→ invalid command name "1"
puts [dict map {k v} {a 1 b 2} {list $v $k}]
puts [dict map {k v} {a 1 b 2} {list $k}]
puts [dict map {k v} {a 1 b 2} {}]
puts [dict filter {a b c d} key]
puts [dict filter {a1 a a2 b b1 c} key a? b?]
puts [dict filter {a 1 b 2} script {k v} {expr {$k eq "a"}}]
puts [dict filter {a 1 b 2 c 3} script {k v} {$v > 1 ? [list $k $v] : {}}]
