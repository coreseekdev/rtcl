proc empty {} { return "" }
set acc 0
if [catch {incr acc [empty]} msg] { puts "E:$msg EC=$errorCode" }
