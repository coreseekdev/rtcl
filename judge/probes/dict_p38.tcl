catch {tcl::mathop::+ {*}[dict map {k v} [lsearch -all [lrepeat 5 x] x] { expr { $k * $v } }]} r
puts "mathop: $r"
puts "lsearch-all: [lsearch -all {x a x} x]"
puts "lrepeat: [lrepeat 3 x]"
set c [catch {apply {{} {tcl::dict::lappend foo bar \n[format baz]}} } r2]
puts "ens2: c=$c r=$r2"
