puts "ret: [catch {return -level 0 -code return rr} a] a=$a"
puts "cont: [catch {for {set i 0} {$i < 3} {incr i} {if {$i == 1} {return -level 0 -code continue}}} b] b=$b i=$i"
set s {}
set r [catch {foreach x {1 2 3} {if {$x == 2} {return -level 0 -code continue}; lappend s $x}} b2]
puts "fc: r=$r s=$s b=$b2"
puts "int7: [catch {return -level 0 -code 7 zz} c] c=$c"
set d {}
set r2 [catch {while {1} {incr d(0); return -level 0 -code error eek}} d2]
puts "werr: r=$r2 d=[array get d] m=$d2"
