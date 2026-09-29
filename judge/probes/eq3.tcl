set r "a b c d d300 d35 e \\{"
if {$r eq {a b c d d300 d35 e \{}} {puts IF-YES} {puts IF-NO}
puts [expr {{a \{} eq {a \{}}]
set w2 {a \{}
puts "vv2=[expr {$w2 eq $w2}] litvs=[expr {$w2 eq {a \{}}]"
puts "sc=[string length {a \{]}] tclset=[string length $w2]"
