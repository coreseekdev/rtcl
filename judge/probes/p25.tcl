puts [switch -exa Foo Foo {subst OK}]
puts [catch {switch -q x a {subst 1}} m]; puts $m
puts [catch {switch x} m]; puts $m
puts [catch {switch -exact x} m]; puts $m
puts [catch {switch b {b -foo}} m]; puts $m
puts [catch {switch a {a - b -foo}} m]; puts $m
puts [switch a {a - b -foo c {subst 3}}]
puts [catch {switch x {a {} b}} m]; puts $m
puts [catch {switch x {a {} b # comment}} m]; puts $m
puts [catch {switch x {# comment}} r]; puts "$r"
puts [switch x {a {} # comment}]
set x BAD
switch -regexp -matchvar x -- "a b c" {bc {list $x YES} default {set x}}
puts "x-now:[$x]"
