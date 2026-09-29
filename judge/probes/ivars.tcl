namespace eval n { variable v 1 }
puts "A:[info vars ::n::*]"
puts "B:[info vars n::*]"
puts "C:[info globals ::n::*]"
puts "D:[info exists ::n::v] [info exists n::v]"
set ::n::w 2
puts "E:[info vars ::n::*]"
puts "F:[namespace eval n { info vars [namespace current]::* }]"
