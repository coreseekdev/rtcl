proc t {fmt str} {
    set r [catch {scan $str $fmt} res]
    if {$r} { puts "$fmt|$str -> ERR $res" } else { puts "$fmt|$str -> $res" }
}
t {%[0-9]} abc123
t {%2[0-9]} abc123
t {%5s} {ab cd}
t %u -1
t %u 4294967296
t %u -4294967296
t %i 010
t %i 018
t %i -018
t %i +5
t %x ffffffffffffffff
t %x 0x
t %i 0x
t %lc a
t %zn 5
t %lls abc
t "%L\[abc\]" abc
t %hc abc
t {%h[set]} x
t %ln 5
t %ld 5
t %f inf
t %f -Infinity
t %f NaN
t %f na
t %g 1x
puts [scan abcdef %0s%n v1 v2]:$v1:$v2
puts [scan abcdef %2s%n v1 v2]:$v1:$v2
puts [scan abcdef %s%n v1 v2]:$v1:$v2
puts [scan x12y %2i%n v1 v2]:$v1:$v2
puts [scan -12 {%2c%n} a b]:$a:$b
puts [catch {scan {} %c v} r]; puts $r
puts [scan {ab} {%[%]} x]:$x
puts [scan {a} {%[^b]c} x y]:$x:$y
puts [scan {aaa} {%2[^b]%c} x y]:$x:$y
