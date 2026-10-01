puts "re=[regexp {^(.)\1} "11" m] <$m>"
puts "re2=[regexp {(a)\1} "aa" m2] <$m2>"
catch {unset a}
set a(11) 1
set a(12) 1
puts "names=[array names a -regexp {^(.)\1}]"
puts "names-glob=[array names a -regexp {^1}]"
