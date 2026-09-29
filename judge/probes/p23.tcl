puts [switch a {a - z {subst 1}}]
puts [switch a {a - z {subst 1} default {subst 2}}]
puts [switch b {a {subst 1} a - b {subst 2}}]
puts [catch {switch a {a - b -}} m]; puts $m
set e [catch {switch foo a {error switch1} b {error switch 3} default {error switch2}} m]
puts "$e|$m"
