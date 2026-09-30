proc tryit {s} {
    set c [catch {dict replace $s} r]
    if {$c == 1} {
        catch {catch {dict replace $s} -> opt; dict get $opt -errorcode} c2 ec
        puts "ERR <$s> | $r | $ec"
    } else { puts "OK  <$s> | $r" }
}
tryit "{"
tryit "a b \""
tryit "{a b}c d"
tryit "a b c"
foreach c {LIST JUNK BRACE QUOTE} { puts "x" }
# list side for comparison
set c [catch {lindex "\{" 0} r]
catch {catch {lindex "\{" 0} -> opt; dict get $opt -errorcode} c2 ec
puts "list-unmatched: $r | $ec"
set c [catch {lindex "{a b}c" 0} r]
catch {catch {lindex "{a b}c" 0} -> opt; dict get $opt -errorcode} c2 ec2
puts "list-junk: $r | $ec2"
set c [catch {dict replace {a b c}} r]
catch {catch {dict replace {a b c}} -> opt; dict get $opt -errorcode} c2 ec3
puts "dict-odd: $r | $ec3"
