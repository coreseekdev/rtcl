# P16: 15M bare-name reads/writes in a namespace body (eval level).
set n 5000000
namespace eval nb { variable v 5 }
namespace eval nb {
    for {set i 0} {$i < $::n} {incr i} { set v }
    for {set i 0} {$i < $::n} {incr i} { set v 7 }
    array set arr {1 x}
    for {set i 0} {$i < $::n} {incr i} { set arr(1) }
}
puts DONE
