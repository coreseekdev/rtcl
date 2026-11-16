set acc 0
if [catch {incr acc [expr {$undefined}]} msg] { puts "E:$msg" } else { puts "ok:$msg" }
catch {incr acc [expr {$undefined}]}
puts $errorInfo
puts $errorCode
