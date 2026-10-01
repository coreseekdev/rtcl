array set x {a 1}
set arrayname x
puts [catch {${arrayname}(a)} r]; puts $r
puts "${arrayname}($arrayname)"
