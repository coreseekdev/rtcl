# proc / scope / return codes
proc greet {name} { return "hi $name" }
puts [greet "world"]
proc add {a b} { expr {$a + $b} }
puts [add 3 4]
proc counter {} {
    set c 0
    incr c
    return $c
}
puts [counter]
puts [counter]
proc varscope {} { set local 1; expr {$local + 1} }
puts [varscope]
# default args
proc defarg {{x 10}} { return $x }
puts [defarg]
puts [defarg 99]
# return value of proc body = last command
proc implicit {} { set y 5 }
puts [implicit]
# upvar
proc setcaller {vname val} { upvar 1 $vname v; set v $val }
set target 0
setcaller target 77
puts $target
# uplevel
proc do1 {script} { uplevel 1 $script }
set z 1
do1 {set z 42}
puts $z
# recursion
proc fact {n} { if {$n <= 1} { return 1 }; expr {$n * [fact [expr {$n-1}]]} }
puts [fact 10]
