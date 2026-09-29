set r [lsort {d e c b a \{ d35 d300}]
if {$r eq {a b c d d300 d35 e \{}} {puts YES} {puts NO}
