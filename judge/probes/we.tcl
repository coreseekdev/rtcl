proc boomq {} {error "boomq-msg"}
catch {set q [boomq]}
puts "F9:[string map {\n |} $::errorInfo]"
catch {puts "A:[boomq]"}
puts "E1:[string map {\n |} $::errorInfo]"
catch {set y [lindex $q [boomq]]}
puts "E2:[string map {\n |} $::errorInfo]"
