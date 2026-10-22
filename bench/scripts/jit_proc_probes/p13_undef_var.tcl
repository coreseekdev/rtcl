proc p {} {puts $nope}
catch {p} m
set m
