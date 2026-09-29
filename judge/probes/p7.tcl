set r [catch {expr {1}junk} m]; puts "brace-junk: $r $m"
set r [catch {set v "ab"cd} m]; puts "set-quote-junk: $r $m"
