set out {}
# int/int binop + cmp
append out [expr {3 + 4}] [expr {10 / 3}] [expr {7 % 3}] [expr {2 < 3}] [expr {3 <= 3}] [expr {5 > 6}] [expr {-8 < 0}] \n
# overflow widen
append out [expr {9223372036854775807 + 1}] \n
append out [expr {-9223372036854775808 - 1}] \n
# bignum cmp vs i64
append out [expr {99999999999999999999 > 9223372036854775807}] \n
append out [expr {99999999999999999999 < 100000000000000000000}] \n
# float mixing
append out [expr {1 + 2.5}] [expr {7 / 2.0}] [expr {2.5 < 3}] [expr {1e300 * 1e300}] \n
# NaN errors (both operand orders and both classes)
append out [catch {expr {nan + 0}} e1] $e1 \n
append out [catch {expr {0 + nan}} e2] $e2 \n
append out [catch {expr {1.0 * nan}} e3] $e3 \n
append out [catch {expr {nan < 3}} e4] $e4 \n
# Inf semantics
append out [expr {1 / 0.0}] [expr {-1 / 0.0}] [expr {inf + 1}] \n
# string operands (the double-parse case)
append out [expr {"12" + "30"}] [expr {"5" < "10"}] \n
# div/mod errors
append out [catch {expr {1 / 0}} e5] $e5 [catch {expr {1 % 0}} e6] $e6 \n
# i64::MIN / -1 and % -1
append out [expr {-9223372036854775808 / -1}] [expr {-9223372036854775808 % -1}] \n
# hex/octal/binary operands
append out [expr {0x10 + 0o7 + 0b101}] \n
# comparisons across rep kinds
append out [expr {2.0 == 2}] [expr {2 == 2.0}] [expr {0x10 > 15}] \n
puts -nonewline $out
