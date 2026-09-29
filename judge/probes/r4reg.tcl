set x 40
puts [catch {apply {{} {
    incr x
    set x
}}} r]
puts "r=<$r>"
puts "global x=[set x]"
