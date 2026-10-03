# 1. plain level-0 return in a proc body
proc p1 {} {return -level 0 x; puts no}
puts "p1: [p1]"
# 2. catch interaction
puts "catch: [catch {return -level 0 x} r] r=$r"
# 3. level 0 + code error
puts "e1: [catch {return -level 0 -code error boom} m] m=$m"
# 4. level 0 + code break
set i 0
set c [catch {while {1} {return -level 0 -code break}} m2]
puts "brk: c=$c m=$m2 i=$i"
# 5. for-next: level-0 return then MORE commands after it in the same next script
set out {}
for {set i 0} {$i < 3} {incr i; return -level 0 q; lappend out after} {lappend out $i}
puts "out=$out"
# 6. while body: level-0 return then trailing command
set w {}
set j 0
while {$j < 2} {return -level 0 v; lappend w tail; incr j}
puts "w=$w j=$j"
# 7. -level 0 with no value
proc p2 {} {return -level 0; puts p2no}
puts "p2: [p2]"
# 8. nested: level-0 return inside eval inside loop body
set ev {}
for {set k 0} {$k < 2} {incr k} {eval {return -level 0 z}; lappend ev $k}
puts "ev=$ev"
