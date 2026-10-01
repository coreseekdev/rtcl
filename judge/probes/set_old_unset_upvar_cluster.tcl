# 7.7
set y 0
proc p1 {} { global y; set z [p2]; return [list $z [catch {set y} msg] $msg] }
proc p2 {} {global y; unset y; list [catch {set y} msg] $msg}
puts "77=[p1]"
# 8.38.3
catch {unset aVaRnAmE}
puts "383=[catch {upvar 0 aVaRnAmE(elem) elemAliAs}] [catch {array set elemAliAs {}} msg] <$msg>"
# 8.38.4
catch {unset bVaR}
array set bVaR [list e1 v1 e2 v2]
array set bVaR {}
puts "384=[lsort [array names bVaR]]"
# 8.52.1
catch {unset a}
set a(1*2) 1
set a(12) 1
set a(11) 1
puts "521=[catch {lsort [array names a -regexp {^(.)\1}]} msg] <$msg>"
