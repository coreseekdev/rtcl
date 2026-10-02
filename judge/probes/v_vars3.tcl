set b 5
catch {unset zz1}; puts "unset-scalar: $errorCode"
catch {unset b(0)}; puts "unset-elem: $errorCode"
catch {append b(1) q}; puts "append-elem-nonarr: $errorCode"
catch {incr b(2)}; puts "incr-elem-nonarr: $errorCode"
catch {string toupper $b(3)}; puts "read-elem-nonarr: $errorCode"
array set a {1 x}
catch {puts $a(zz)}; puts "read-elem-missing: $errorCode"
catch {unset a(zz)}; puts "unset-elem-missing: $errorCode"
catch {lappend a(zz) q}; puts "lappend-elem-missing: $errorCode"
