set out {}
dict filter {a 1 b 2 c 3} script {k v} {lappend out $k; if {$k eq "b"} {break}; expr 1}
puts $out
set out {}
dict map {k v} {a 1 b 2 c 3} {lappend out $k; if {$k eq "b"} {continue}; string cat $v}
puts $out
puts [catch {dict filter {a b} JUNK} m]; puts $m
puts [catch {dict filter {a b c} key} m]; puts $m
puts [catch {dict map "\{x" x x} m]; puts $m
puts [catch {dict filter {a b} script "\{k v" {continue}} m]; puts $m
puts [catch {dict filter {a 1 b 2} script {k v} {list $k $v}} m]; puts $m
puts [dict filter {a 1 b 2} script {k v} {expr {$v >= 2}}]
puts [dict map {k v} {a 1} {expr {$v + 1}}]
