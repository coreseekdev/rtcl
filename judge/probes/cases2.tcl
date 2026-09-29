set r [lsort {d e c b a \{ d35 d300}]
puts "eq=[expr {$r eq {a b c d d300 d35 e \{}}] len=[string length $r]"
puts "want=[string length {a b c d d300 d35 e \{}}]"
puts [list R3 [catch {lsort -index {} [list a \{]} m] $m]
set q [list a \{]
puts [list R3b [catch {lsort -index {} $q} m2] $m2]
