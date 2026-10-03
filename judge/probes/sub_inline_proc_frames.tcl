proc p {} {
    set x [string length [foo $nope]]
}
p
