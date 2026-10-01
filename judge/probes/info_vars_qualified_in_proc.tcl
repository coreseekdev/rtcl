namespace eval ::ref {}
set ::ref::var1 AAA
set ::ref::var2 BBB
proc q {} { return [info vars ::ref::*] }
puts "in-proc=<[q]> tclsh-next"
puts "global=<[info vars ::ref::*]>"
proc r {} { return [info vars ::ref::*] }
