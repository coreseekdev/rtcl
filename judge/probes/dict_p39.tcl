set m [dict map {k v} [lsearch -all [lrepeat 100000 x] x] { expr { $k * $v } }]
puts "map-done: [dict size $m]"
set s [tcl::mathop::+ {*}$m]
puts "sum: $s"
