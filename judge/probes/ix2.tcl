proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t n-008 {string index abcdefgh 008}
t n-08 {string index abcdefgh 08}
t n-i64max {string index abcdefgh 9223372036854775807}
t n-i64min {string index abcdefgh -9223372036854775808}
t n-max6 {string index abcdefgh 9223372036854775806+1}
t n-maxd {string index abcdefgh 9223372036854775807-9223372036854775807}
t n-endmax {string index abcdefgh end+9223372036854775807}
t n--plus1 {string index abcdefgh -+1}
t n-1pm1 {string index abcdefgh 1+-1}
t n-end-ws2 {string index abcdefgh { end - 1 }}
t n-END {string index abcdefgh END}
t n-plus {string index abcdefgh +}
t li-bad {lindex {a b} 1+}
t lr-bad {lrange {a b} x+1 1}
t sr-bad {string range abc x+1 2}
t li-empty {lindex {} 0+}
t ls-bad {lsearch {a b} -index 1+ x}
t li-multi {lindex {{a b} c} 0+0 1+0}
t cx-invalid {concat a {b{c}}
t cx-invalid2 {concat a "b\"c}
t cx-bsbrace {concat a {b\}c} d}
t lst-tab {list "a\tb"}
t lst-nl {list "a\nb"}
t lst-cr {list "a\rb"}
t lst-bsnl {list "a\\\nb"}
t lst-semi {list {a;b}}
t lst-lbrack {list {a[b}c}
t concat-elem-space-emit {concat [list "a b"] [list c]}
t concat-tab-elem {concat [list "a\tb"] c}
t concat-nl-elem {concat [list "a\nb"] c}
