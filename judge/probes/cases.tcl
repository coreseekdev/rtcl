puts [list R1 [catch {lsort {d e c b a \{ d35 d300}} m] [list $m]]
puts [list R2 [catch {lsort -index 1+3 {{1 . c} {2 . b} {3 . a}}} m] [list $m]]
puts [list R3 [catch {lsort -index {} [list a \{}} m] [list $m]]
puts [list R4 [catch {lsort -index 1 {a b} } m] [list $m]]
