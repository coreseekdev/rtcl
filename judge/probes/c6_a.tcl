set list {1 2 3 4 5}
set index 0
set result {}
while {$index < 5} {
    if {$index == 3} break
    set result [concat $result [lindex $list $index]]
    set index [expr {$index + 1}]
}
set result
