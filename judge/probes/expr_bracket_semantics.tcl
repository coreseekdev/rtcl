set a [expr {[expr {6 * 7}] + 1}]
set b [expr {[list a b] eq [list a b]}]
set c [expr {[llength [list 1 2 3]] * 2}]
proc f x { expr {$x + 10} }
set d [expr {[f 5] + [f 6]}]
set e [expr {-[f 0]}]
set g [expr {([f 1]) + 2}]
proc h {} { list 1 2 }
set i [expr {[llength [h]] == 2}]
set r "$a $b $c $d $e $g $i"
puts $r
