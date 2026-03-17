# Tests for {*} expand (CallExpand fix)
source tests/rtcl_test.tcl

# Test 1: basic expand with list command
set mylist {b c d}
test expand-1 "list with expand" {list a {*}$mylist e} {a b c d e}

# Test 2: expand with puts-like (DynCall path)
proc capture {args} {
    return $args
}
set items {x y z}
test expand-2 "proc with expand" {capture {*}$items} {x y z}

# Test 3: expand empty list
set empty {}
test expand-3 "expand empty list" {list a {*}$empty b} {a b}

# Test 4: expand single-element list
set single {hello}
test expand-4 "expand single element" {list {*}$single} hello

# Test 5: multiple expands
set l1 {a b}
set l2 {c d}
test expand-5 "multiple expands" {list {*}$l1 {*}$l2} {a b c d}

# Test 6: expand with string command (CmdId path)
set words {hello world}
test expand-6 "string cat with expand" {string cat {*}$words} helloworld

testreport
