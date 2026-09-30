set d "{a b}c"
set c [catch {dict set d k v} r]; puts "set-junk: c=$c r=$r"
set d2 {a b c}
set c [catch {dict set d2 k v} r]; puts "set-odd: c=$c r=$r"
set c [catch {dict unset d2 c} r]; puts "unset-odd: c=$c r=$r"
set c [catch {dict append d2 k v} r]; puts "append-odd: c=$c r=$r"
set c [catch {dict for "\{x" x {a b}} r]; puts "for-order: c=$c r=$r"
set c [catch {dict map "\{x" x {a b}} r]; puts "map-order: c=$c r=$r"
set c [catch {dict exists {a b} c d} r]; puts "exists-nested-missing: c=$c r=$r"
set c [catch {dict get {a b} c d} r]; puts "get-nested-missing: c=$c r=$r"
set d3 {a b}
set c [catch {dict unset d3 a b} r]; puts "unset-nested-leaf: c=$c r=$r"
