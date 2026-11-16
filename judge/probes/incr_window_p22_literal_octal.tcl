set acc 0
incr acc 007
puts $acc
incr acc +1
puts $acc
if [catch {incr acc 1a} msg] { puts "E:$msg" }
