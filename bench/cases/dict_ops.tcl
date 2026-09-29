# dict-heavy workload
set d [dict create]
for {set i 0} {$i < 3000} {incr i} {
    dict set d "key$i" $i
}
set acc 0
foreach k [dict keys $d] {
    incr acc [dict get $d $k]
}
puts "$acc [dict size $d]"
