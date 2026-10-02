catch {unset nope}; puts "unset: $errorCode"
catch {set nope}; puts "set: $errorCode"
catch {puts $nope}; puts "read: $errorCode"
catch {incr nope}; puts "incr: $errorCode"
array set a {1 x}
catch {puts $a(zz)}; puts "elem: $errorCode"
