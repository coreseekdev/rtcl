set d {a 1 b 2}
catch {dict with d {error boom}}
puts "on-error exists-a:[info exists a] d=$d"
set d2 {p 5}
puts [catch {dict with d2 p {zz}} m]; puts $m
set d3 {a 1}
dict with d3 {unset a; unset d3}
puts "end: [info exists d3] [info exists a]"
