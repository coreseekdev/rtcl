# control flow: if/while/for/foreach/switch
if {1 < 2} { puts "yes" } else { puts "no" }
if {1 > 2} { puts "yes" } elseif {2 > 1} { puts "elseif" } else { puts "no" }
set i 0
while {$i < 3} { puts "w$i"; incr i }
for {set j 0} {$j < 3} {incr j} { puts "f$j" }
foreach x {a b c} { puts "e$x" }
foreach {a b} {{1 2} {3 4}} { puts "$a-$b" }
set n 0
foreach x {1 2 3 4 5} { if {$x == 3} continue; if {$x == 5} break; incr n }
puts "n=$n"
switch abc {
    a { puts "A" }
    abc { puts "matched" }
    default { puts "D" }
}
switch -glob "hello.txt" {
    *.tcl { puts "tcl" }
    *.txt { puts "txt" }
}
