puts [switch a {a - b {subst 2}}]
puts [switch a {a - b - c {subst 9}}]
puts [switch a {a - default {subst 7}}]
puts [catch {switch a {a - b}} m]; puts $m
puts [switch x {a {subst 1} a - b {subst 2}}]
set e [catch {switch a a {set y 1
error E2}} m]; puts "$e|$m"
puts $::errorInfo
