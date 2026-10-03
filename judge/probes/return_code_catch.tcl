puts "plain: [catch {return -code error boom} m] m=$m"
puts "lvl0: [catch {return -level 0 -code error boom} m2] m2=$m2"
set r [catch {while {1} {return -code break}} m3]
puts "plainbrk: c=$r m=$m3"
