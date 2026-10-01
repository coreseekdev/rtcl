catch {unset a}
puts "upvar=[catch {upvar 0 a(e) x} m]<$m>"
puts "exists_x=[info exists x]"
puts "exists_a=[info exists a]"
puts "array_a=[array exists a]"
puts "a_get=[catch {array get a} m]<$m>"
puts "set_x=[catch {set x 5} m]<$m>"
puts "read_a_e=[catch {set a(e)} v]<$v>"
