puts [catch {switch x {a {} b # comment}} m]; puts $m
puts [catch {switch x {# comment}} r]; puts "$r"
puts [switch x {a {} # comment}]
puts [catch {switch -exact -glob x a {b}} m]; puts $m
puts [catch {switch -matchvar v x a {b}} m]; puts $m
puts [catch {switch -indexvar} m]; puts $m
set x BAD
switch -regexp -matchvar x -- "a b c" {bc {list $x YES} default {set x}}
puts "x-now:\[$x\]"
puts [catch {switch a {a - default {subst 7} z {subst 8}}} m]; puts $m
