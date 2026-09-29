catch {unset x}
set errorCode NONE
puts "A:[info globals ::err*]"
puts "B:[info globals err*]"
puts "C:[info vars ::err*]"
puts "D:[info vars [namespace current]err*]"
namespace eval q { variable loc 1 }
puts "E:[info globals q::*]"
puts "F:[info vars q::*]"
proc pp {} { variable up1 7; puts "G:[info vars up*]" }
pp
puts "H:[info exists q::loc] [info exists loc]"
