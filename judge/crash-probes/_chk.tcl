# Shared assertion helper for judge/crash-probes/*.tcl:
#   chk <name> <got> <want>   — prints "ok <name>", exits 1 on mismatch.
proc chk {name got want} {
    if {$got ne $want} {
        puts "FAIL $name: got [string range $got 0 80] want [string range $want 0 80]"
        exit 1
    }
    puts "ok $name"
}
