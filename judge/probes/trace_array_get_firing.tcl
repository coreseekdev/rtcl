# does array get fire whole-array read trace, per-element, or both?
set fired {}
proc tr {args} { lappend ::fired $args }
unset -nocomplain x
set x(a) 1
set x(b) 2
trace add variable x read tr
puts "get=[array get x]"
puts "fired=$fired"
set fired {}
puts "names=[array names x]"
puts "fired=$fired"
set fired {}
puts "size=[array size x]"
puts "fired=$fired"
set fired {}
puts "exists=[array exists x] get-with-pattern=[array get x b]"
puts "fired=$fired"
