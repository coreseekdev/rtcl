set e [catch {switch a a {error "Just a test"} default {subst 1}} m]
puts "$e|$m"
puts "INFO: $::errorInfo"
set e2 [catch {proc p {} {error boom}} m2]
puts "P:$e2"
puts "INFO2: $::errorInfo"
