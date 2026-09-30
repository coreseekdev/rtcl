puts [catch {scan a {%}} r]; puts $r
puts [scan {ab} {%s%n} a b]; puts "$a $b"
puts [scan 12 {%2dn} c]; puts $c
