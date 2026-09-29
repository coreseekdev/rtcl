# Tcl list-quoting edge cases — the parser's hardest job
puts [llength "a b c"]
puts [llength "a {b c} d"]
puts [llength {a "b c" d}]
puts [lindex "a\tb\nc" 1]
puts [llength ""]
puts [llength {{}}]
puts [llength {a {b {c d}} e}]
puts [lindex {{a b} {c d}} 0 1]
puts [llength "a;b;c"]
set x "a b"
puts [llength $x]
puts [llength [list $x]]
# braces vs quotes in substitution
set v 42
puts "{v=$v}"
puts "v=$v"
puts [list a $v {b c}]
# semicolons and newlines as command separators
set a 1; set b 2; puts [expr {$a+$b}]
# comment handling
puts before ;# trailing comment
# full-line comment
puts after
