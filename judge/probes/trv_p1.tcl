unset -nocomplain x
trace add variable x write {traceTag 1}
trace add variable x write traceProc
unset -nocomplain x
puts "after-unset-info:[trace info variable x]"
trace add variable x write foo
trace remove variable x write foo
puts "llength:[llength [trace info variable x]]"
unset -nocomplain x
set x 44
trace add variable x(0) write traceProc
puts "12.8:[list [catch {trace add variable x(0) write traceProc} msg] $msg]"
unset -nocomplain x
set x 44
trace add variable x write {traceTag 1}
puts "14.19:[trace info variable x(0)]"
