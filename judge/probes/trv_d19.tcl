proc cb {args} { puts "ARGS: [llength $args] <[join $args ,]>" }
set r1 [catch {namespace inscope :: cb alpha {} delete} m1]
puts "r1=$r1 m1=$m1"
set r2 [catch {namespace inscope :: cb {a b} {c d}} m2]
puts "r2=$r2 m2=$m2"
set r3 [catch {namespace inscope :: cb} m3]
puts "r3=$r3 m3=$m3"
set r4 [catch {namespace inscope :: cb x y z w} m4]
puts "r4=$r4 m4=$m4"
