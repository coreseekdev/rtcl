proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t s1 {expr {sqrt("1[string repeat 0 616]") == 1e308}}
t s2 {expr {sqrt("1[string repeat 0 616]")}}
t s3 {expr {"1[string repeat 0 400]" == 1e308}}
t s4 {expr {double(0x7fffffffffffffff)}}
t s5 {expr {round(3.7)}}
t s6 {expr {pow(2.0,0.5)}}
t s7 {expr {entier(3.7)}}
t s8 {expr {double(9223372036854775807)}}
t s9 {expr {round(double(9223372036854775807))}}
