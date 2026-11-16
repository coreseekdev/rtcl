proc p {} { set a 0; incr a [return 7]; return $a }
puts [p]
set b 0
incr b [return 3]
puts $b
