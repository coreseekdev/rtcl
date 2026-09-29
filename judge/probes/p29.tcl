puts [catch {dict with nodict {zz}} m]; puts $m
set d {a 1}
dict with d {error boom}
puts "after-error: [info exists a] | $d"
set d {p {q 1} r 5}
dict with d p {set q 9}
puts $d
set d {a 1 b 2}
dict with d {unset a}
puts "unset-via-with: $d"
