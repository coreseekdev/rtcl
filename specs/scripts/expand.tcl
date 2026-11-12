# {*}$args 展开语义——编译路径（-f 整脚本）。元素应整词传递。
# eval/树走路径见 specs/expand.probes（11/11）。
proc src {args} { return $args }
set L [src {a b {c d}}]
puts [list [llength $L] [llength [lindex $L 0]]]
proc sink {args} { return [llength $args] }
puts [sink {*}$L]
proc sink2 {args} { return $args }
set R [sink2 {*}$L]
puts [list [llength $R] [lindex $R 0]]
set d {k {v 1 w 2}}
set R2 [sink {*}[list $d]]
puts [list [llength $R2] [llength [lindex $R2 0]]]
set L3 [src {{ "a": 1, "b": [2, 3] }}]
puts [sink {*}$L3]
proc sink6 {args} { return [lindex $args 0] }
puts [sink6 {*}$L3]
proc inner {args} { return $args }
proc outer {args} { return [inner {*}$args] }
set R3 [outer {p q {r s}}]
puts [list [llength $R3] [llength [lindex $R3 0]]]
proc sink9 {args} { return $args }
set R4 [sink9 {*}[list {{ "x": 1.50 }}]]
puts [list [llength $R4] [lindex $R4 0]]
set L5 [src {a b {c d}}]
puts [sink {*}$L5]
puts [sink {*}[src {x y z}]]
