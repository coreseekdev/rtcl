puts [info exists errorCode]
catch {error boom} m
puts "$m | $errorCode"
catch {nosuchcmd} m2
puts "$m2 | $errorCode"
proc f {a b} {return ok}
catch {f x} m3
puts "$m3 | $errorCode"
catch {expr {1/0}} m4
puts "$m4 | $errorCode"
set e2 [catch {lsort -integer {09 8}}]
puts "$e2 | $errorCode"
catch {unset nope} m6
puts "$m6 | $errorCode"
