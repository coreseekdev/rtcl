# Tests for frame stack scoping
source tests/rtcl_test.tcl

# Test 1: proc has its own scope (can't see global vars without global cmd)
set gval 42
proc test_scope {} {
    catch {set gval} err
    return $err
}
test scope-1 "proc has separate scope" {test_scope} {can't read "gval": no such variable}

# Test 2: global command gives access to globals
set gval2 99
proc test_global {} {
    global gval2
    return $gval2
}
test scope-2 "global makes var visible" {test_global} 99

# Test 3: global command allows writing globals
proc test_global_write {} {
    global gval3
    set gval3 wrote_from_proc
}
test_global_write
test scope-3 "global allows writing" {set gval3} wrote_from_proc

# Test 4: upvar links to caller's variable
proc do_incr {varname} {
    upvar 1 $varname local
    set local [expr {$local + 1}]
}
set counter 10
do_incr counter
test scope-4 "upvar links to caller" {set counter} 11

# Test 5: uplevel evaluates in caller's scope
proc run_in_caller {script} {
    uplevel 1 $script
}
set x 100
run_in_caller {set x 200}
test scope-5 "uplevel runs in caller scope" {set x} 200

# Test 6: nested proc calls have separate scopes
proc outer {} {
    set local_val outer_val
    inner
}
proc inner {} {
    catch {set local_val} err
    return $err
}
test scope-6 "nested procs have separate scopes" {outer} {can't read "local_val": no such variable}

# Test 7: info level returns frame depth
proc depth {} {
    return [info level]
}
test scope-7 "info level inside proc" {depth} 1

# Test 8: upvar #0 links to global
proc test_upvar_global {} {
    upvar #0 globalref localref
    set localref from_upvar_global
}
test_upvar_global
test scope-8 "upvar #0 links to global" {set globalref} from_upvar_global

# Test 9: the test framework itself (global + uplevel) works correctly
test scope-9 "test framework uses global+uplevel" {expr {1 + 1}} 2

testreport
