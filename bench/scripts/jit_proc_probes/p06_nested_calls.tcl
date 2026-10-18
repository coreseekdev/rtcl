proc outer {} {return [inner 3]}
proc inner {x} {return [expr {$x * 7}]}
puts [outer]
