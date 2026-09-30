set x GLOBAL
namespace eval t1 {variable x; puts "bare:[catch {set x} m] $m"}
namespace eval t2 {variable x 5; unset x; puts "afterunset:[catch {set x} m] $m"; puts "exists:[info exists x]"; set x 7; puts "recreate:$x [namespace which -variable x]"}
namespace eval t3 {variable y 5; puts "valset:[set y]"}
puts "t2.which:[namespace eval t2 {namespace which -variable x}]"
