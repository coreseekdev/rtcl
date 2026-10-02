catch {puts $zz(5)}; puts "missing-arr-elem: $errorCode"
catch {info exists zz(5)}; puts "exists: $errorCode"
