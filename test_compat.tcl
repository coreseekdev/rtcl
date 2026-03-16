# Simple compatibility test for rtcl

# Test basic commands
puts "Testing rtcl compatibility..."

# Test 1: Basic variables
set a 1
if {$a == 1} {
    puts "Test 1 PASS: Basic variables"
} else {
    puts "Test 1 FAIL: Basic variables"
}

# Test 2: List operations
set lst {a b c}
if {[llength $lst] == 3} {
    puts "Test 2 PASS: llength"
} else {
    puts "Test 2 FAIL: llength"
}

# Test 3: lindex
set val [lindex $lst 1]
if {$val eq "b"} {
    puts "Test 3 PASS: lindex"
} else {
    puts "Test 3 FAIL: lindex (got: $val)"
}

# Test 4: proc
proc test_proc {x} {
    return [expr $x + 1]
}
set result [test_proc 5]
if {$result == 6} {
    puts "Test 4 PASS: proc"
} else {
    puts "Test 4 FAIL: proc (got: $result)"
}

# Test 5: expr
set e [expr 2 + 3 * 4]
if {$e == 14} {
    puts "Test 5 PASS: expr"
} else {
    puts "Test 5 FAIL: expr (got: $e)"
}

# Test 6: foreach
set sum 0
foreach i {1 2 3 4 5} {
    set sum [expr $sum + $i]
}
if {$sum == 15} {
    puts "Test 6 PASS: foreach"
} else {
    puts "Test 6 FAIL: foreach (got: $sum)"
}

# Test 7: while loop
set i 0
set count 0
while {$i < 5} {
    incr count
    incr i
}
if {$count == 5} {
    puts "Test 7 PASS: while"
} else {
    puts "Test 7 FAIL: while (got: $count)"
}

# Test 8: for loop
set total 0
for {set i 0} {$i < 5} {incr i} {
    set total [expr $total + $i]
}
if {$total == 10} {
    puts "Test 8 PASS: for"
} else {
    puts "Test 8 FAIL: for (got: $total)"
}

# Test 9: if/elseif/else
set x 2
if {$x == 1} {
    set result "one"
} elseif {$x == 2} {
    set result "two"
} else {
    set result "other"
}
if {$result eq "two"} {
    puts "Test 9 PASS: if/elseif/else"
} else {
    puts "Test 9 FAIL: if/elseif/else (got: $result)"
}

# Test 10: list command
set mylist [list a b c "d e"]
if {[llength $mylist] == 4} {
    puts "Test 10 PASS: list with spaces"
} else {
    puts "Test 10 FAIL: list with spaces (got: [llength $mylist])"
}

# Test 11: lappend
lappend mylist f
if {[llength $mylist] == 5} {
    puts "Test 11 PASS: lappend"
} else {
    puts "Test 11 FAIL: lappend (got: [llength $mylist])"
}

# Test 12: catch
set status [catch {error "test error"} msg]
if {$status != 0} {
    puts "Test 12 PASS: catch"
} else {
    puts "Test 12 FAIL: catch"
}

# Test 13: info exists
if {[info exists mylist]} {
    puts "Test 13 PASS: info exists"
} else {
    puts "Test 13 FAIL: info exists"
}

# Test 14: string commands
set s "Hello World"
if {[string length $s] == 11} {
    puts "Test 14 PASS: string length"
} else {
    puts "Test 14 FAIL: string length (got: [string length $s])"
}

puts ""
puts "Compatibility test complete!"
