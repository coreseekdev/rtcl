proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t b-intmax {string index abcdefgh 2147483647}
t b-intmaxp1 {string index abcdefgh 2147483648}
t b-intmaxm1 {string index abcdefgh 2147483647-1}
t b-intminp100 {string index abcdefgh -2147483648+100}
t b-intmaxp1op {string index abcdefgh 2147483647+1}
t b-3e9 {string index abcdefgh 3000000000}
t b-0x7fffffff {string index abcdefgh 0x7fffffff}
t b-0x80000000 {string index abcdefgh 0x80000000}
t b-o0x {string index abcdefgh 0x}
t b-o0b2 {string index abcdefgh 0b2}
t b-ws-octal {string index abcdefgh { 008 }}
t b-plus008 {string index abcdefgh +008}
t b-end-p008 {string index abcdefgh end+008}
t li-repeat {string repeat ab 1+1}
t li-first {string first c abcdef 1+1}
t li-wordstart {string wordstart "a b c" 1+1}
t li-lsort {lsort -index 1+0 {{a b}}}
t li-linsert {linsert {a b c} 1+0 X}
t li-lreplace {lreplace {a b c} 1+0 1+0 X}
t li-toplevel {lindex {a b c} end+0}
t sr-oob {string range ab 2147483647 2147483647}
t sr-oob2 {string range ab end+1 end+2}
