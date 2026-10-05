# 1. memo hit path with variable declared
oo::class create C {
    variable acc
    method compute {x y} { set acc [expr {($x + $y) % 1000}]; return $acc }
}
C create c
set r1 [list [c compute 7 3] [c compute 8 5] [c compute 1 2]]
# 2. variable added at runtime: new link target visible (a fresh var, unset first)
oo::define C variable extra
oo::define C method usex {} { set extra 5; set acc [expr {$acc + $extra}] }
set r2 [c usex]
# 3. param name shadows a variable name -> filtered from link prefix
oo::class create D {
    variable p
    method m {p} { set p [expr {$p * 3}]; return $p }
}
D create d
set r3 [d m 4]
# 4. redefine method after memo filled
oo::define C method compute {x y} { expr {$x - $y} }
set r4 [c compute 10 3]
# 5. variable list ORDER matters (snapshot compare) - add then verify link set
oo::class create E { variable a; method s {} { set a 1; set a } }
E create e
set r5 [e s]
oo::define E variable b
oo::define E method s2 {} { set b 2; return $b }
set r6 [e s2]
# 6. objdefine variable on an object (Owner::Object path)
oo::class create F { method g {} { return g1 } }
F create f
oo::objdefine f variable ov
oo::objdefine f method h {} { set ov 9; return $ov }
set r7 [f h]
# 7. next into an inherited method that declares variables
oo::class create G1 { variable gv; method n {} { set gv 1; return g1-$gv } }
oo::class create G2 { superclass G1; method n {} { return [next]-g2 } }
G2 create g2
set r8 [g2 n]
list $r1 $r2 $r3 $r4 $r5:$r6 $r7 $r8
puts \[list $r1 $r2 $r3 $r4 $r5:$r6 $r7 $r8\]
oo::class create K {
    variable total log
    constructor {seed} { set total $seed; set log {} }
    method add x { incr total $x; lappend log $x; return $total }
    method hist {} { return $log }
    destructor { global dead; lappend dead "t=$total" }
}
set dead {}
K create k 100
set r1 [k add 5]
set r2 [k add 6]
set r3 [k hist]
k destroy
set r4 $dead
puts [list $r1 $r2 $r3 $r4]
