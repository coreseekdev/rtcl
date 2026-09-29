puts [info patchlevel]
set ch [format %c 128336]
puts "astral char len=[string length $ch] cp=[scan $ch %c]"
set adlam [format %c 0x1E950]
puts "adlam cp=[scan $adlam %c] digit=[string is digit $adlam]"
puts "devanagari digit ९ digit=[string is digit ९]"
puts "mathematical digit 𝟘 U+[format %04X [scan 𝟘 %c]] digit=[string is digit 𝟘]"
# -failindex semantics
puts [string is digit -failindex pos 12a34]; puts "pos=$pos"
puts [string is digit -failindex pos2 1234]; puts "pos2=$pos2"
puts [string is digit -failindex pos3 {}]; puts "pos3=$pos3"
catch {string foo bar} m; puts $m
catch {string tolower} m; puts $m
catch {string toupper a b} m; puts $m
catch {string trim a b c d} m; puts $m
catch {string match} m; puts $m
catch {string map abc x} m; puts $m
catch {string map {a b c} x} m; puts $m
catch {string repeat a} m; puts $m
catch {string repeat a x} m; puts $m
catch {string first a} m; puts $m
catch {string replace a 1} m; puts $m
catch {string index a} m; puts $m
catch {string range a 1} m; puts $m
catch {string equal a} m; puts $m
catch {string equal -bogus a b} m; puts $m
catch {string equal -length x a b} m; puts $m
catch {string compare -length -3 ab cd} m; puts "cmpneg=$m"
