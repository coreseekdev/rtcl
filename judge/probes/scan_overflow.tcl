unset -nocomplain v n
set r [scan {NaN} {%f%n} v n]
puts "NaN-only: r=$r v=[info exists v] n=[info exists n]"
unset -nocomplain v n
set r [scan {nan} {%f%n} v n]
puts "nan-only: r=$r v=[info exists v] n=[info exists n]"
unset -nocomplain w
set r [scan {fffffffffffffffff} %x w]; puts "x17f: r=$r w=$w"
set r [scan {-ffffffffffffffff} %x w]; puts "x-16f: r=$r w=$w"
set r [scan {9999999999999999999} %d w]; puts "d19: r=$r w=$w"
set r [scan {0xffffffffffffffff} %i w]; puts "i16f: r=$r w=$w"
set r [scan {377777777777777777777} %o w]; puts "o21: r=$r w=$w"
set r [scan {0x10000000000000000} %x w]; puts "x2p64: r=$r w=$w"
