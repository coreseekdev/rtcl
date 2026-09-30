proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t p01 {expr {0**1}}
t p02 {expr {wide(0)**wide(1)}}
t p03 {expr {round(double(0x7fffffffffffffff))}}
t p04 {expr {-0x8000000000000001 >> 0x8000000000000000}}
t p05 {expr {sqrt("1[string repeat 0 616]" == 1e308)}}
t p06 {expr {-2**2}}
t p07 {expr {isqrt([expr {1<<2048}]+1)}}
t p08 {expr {entier(pow(double(0x7fffffffffffffff),(1.0/2)))}}
t p09 {expr {-(3037000499) ** 2}}
