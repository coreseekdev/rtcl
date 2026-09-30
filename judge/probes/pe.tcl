set exprs {
    {"abcdefghijklmnopqrstuvwxyz"@0}
    {0@"abcdefghijklmnopqrstuvwxyz"}
    {"abcdefghijklmnopqrstuvwxyz"@"abcdefghijklmnopqrstuvwxyz"}
    {123456789012345678901234567890*"abcdefghijklmnopqrstuvwxyz}
    {aaaaaaaaaa@}
    {aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@}
    {aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@}
    {aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@}
    {aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@}
    {bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb@cccccc}
    {@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}
}
foreach e $exprs {
    if {[catch {expr $e} r]} {
        set lines [split $r \n]
        puts "LEN=[string length $e] ERR1=[lindex $lines 0]"
        puts "   ANNOT=[lindex $lines 1]"
    } else { puts "LEN=[string length $e] OK $r" }
}
