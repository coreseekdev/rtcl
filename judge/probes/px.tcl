foreach e { {0&|0} {0^^0} {0|&0} {a(1+,0)} } {
    if {[catch {expr $e} m]} { puts "ERR: $m" } else { puts "OK: $m" }
}
catch {expr {123456789012345678901234567890*"foo$bar([abcdefghijklmnopqrstuvwxyz)"}} m; puts "47: $m"
catch {expr {123456789012345678901234567890*$bar(abcdefghijklmnopqrstuvwxyz}} m; puts "52: $m"
