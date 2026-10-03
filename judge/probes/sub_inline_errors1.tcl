# nested brackets + errors, multi-line brackets, control flow in brackets
proc p {} {
    set x [foo]
}
proc q {} {
    set x [string length [foo $nope]]
}
set a [if {1} {error boom}]
