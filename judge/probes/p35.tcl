# malformed dict strings: message + errorCode
foreach s {{ a b {}c d } { a b ""c d } {" a b \"c d "} {a b c} {a b {c}} {a {b} c} {a b c d} "\{" "a b \"" "{a b}c d" "a\tb c d"} {
    set c [catch {dict replace $s} r]
    if {$c} {
        catch {catch {dict replace $s} -> opt; dict get $opt -errorcode} c2 e2
        puts "ERR: <$s> msg=$r code=$e2"
    } else {
        puts "OK:  <$s> -> <$r>"
    }
}
# dict merge single arg and empty second
puts "m1:[dict merge { a b c d }]"
puts "m2:[dict merge { a b c d } {}]"
puts "m3:[dict merge {} { a b c d }]"
puts "m4:[dict merge { a b c d } { e f }]"
# dict unset missing key
set d {a b}
set c [catch {dict unset d c} r]; puts "unset-missing: c=$c r=$r"
set c [catch {dict unset d c d} r]; puts "unset-missing2: c=$c r=$r"
# tcl::dict:: ensemble existence
foreach cmd {tcl::dict::lappend tcl::dict::incr tcl::dict::create tcl::dict::set} {
    puts "$cmd: [expr {[info commands $cmd] ne ""}]"
}
catch {tcl::dict::lappend foo bar [format baz]} r; puts "ens-lappend: $r"
catch {tcl::dict::incr foo2 bar} r; puts "ens-incr: $r"
# return -level 0 inside dict map body
puts "r0:[dict map {k v} [dict map {k v} {a 1 b 2 c 3 d 4} { list $v $k }] { return -level 0 "$k,$v" }]"
catch {return -level 0 xyz} r; puts "top-catch-r0: c=[catch {return -level 0 xyz} rr] r=$rr"
# dict for varlist strict
set c [catch {dict for "\{x" x x} r]; puts "for-badvarlist: c=$c r=$r"
