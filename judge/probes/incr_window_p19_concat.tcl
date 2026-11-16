set acc 0
if [catch {incr acc 5[expr {}]} msg] { puts "E1:$msg" }
if [catch {incr acc "5"} msg] { puts "ok:$acc" }
