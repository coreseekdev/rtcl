set acc 0
if [catch {incr acc [string range abc 0 0]} msg] { puts "E:$msg EC=$errorCode" }
catch {incr acc [string range abc 0 0]}
puts $errorInfo
