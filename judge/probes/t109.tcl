catch {unset a}
set a(a) 1
set x [array startsearch a]
puts [catch {array next a s-1-b} msg]; puts $msg
puts [catch {array next a s-1ba} msg2]; puts $msg2
puts [catch {array next a s-9-a} msg3]; puts $msg3
