proc p {} {
    foreach n {A a B b} { set $n 1 }
    set r1 [info locals]
    unset A
    set A 2
    lappend r1 [info locals]
    unset b
    set b 2
    lappend r1 [info locals]
    return $r1
}
puts [p]
proc q {} {
    # compiled locals: declared by literal reference
    set A 1; set a 1; set B 1; set b 1
    set r1 [info locals]
    unset A
    set A 2
    lappend r1 [info locals]
    return $r1
}
puts [q]
proc r {} {
    variable vv
    foreach n {A a} { set $n 1 }
    lappend ::g [info locals]
    unset A
    set A 2
    lappend ::g [info locals]
}
set g {}
r
puts $g
puts DONE
