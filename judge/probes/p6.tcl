if {[catch {expr "0005"zxy} m]} {puts "1 ERR: $m"} else {puts "1 ok: $m"}
if {[catch {set x [list a]} m]} {puts "x ERR: $m"}
if {[catch {puts-{a}c} m]} {puts "2 ERR: $m"}
