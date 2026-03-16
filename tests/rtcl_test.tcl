# Simple test framework for rtcl
# Provides basic test command and test reporting

set test_count 0
set pass_count 0
set fail_count 0

proc test {name description script expected} {
    global test_count pass_count fail_count

    incr test_count
    set result [catch {uplevel 1 $script} actual]

    if {$result != 0} {
        # Script threw an error
        if {$expected eq "1" && $result == 1} {
            incr pass_count
            puts "PASS: $name - $description (error as expected)"
        } else {
            incr fail_count
            puts "FAIL: $name - $description"
            puts "  Expected error, got: $actual"
        }
    } elseif {$actual eq $expected} {
        incr pass_count
        puts "PASS: $name"
    } else {
        incr fail_count
        puts "FAIL: $name - $description"
        puts "  Expected: $expected"
        puts "  Got: $actual"
    }
}

proc testreport {} {
    global test_count pass_count fail_count
    puts ""
    puts "=== Test Report ==="
    puts "Total: $test_count"
    puts "Passed: $pass_count"
    puts "Failed: $fail_count"
    if {$fail_count == 0} {
        puts "All tests passed!"
    }
}
