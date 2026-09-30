proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t idx-1plus1plus1 {string index abcdefgh 1+1+1}
t idx-hexchain {string index abcdefgh 0x1+1}
t idx-endhex {string index abcdefgh end+0x1}
t idx-ws-inner {string index abcdefgh {1 + 1}}
t idx-ws-inner2 {string index abcdefgh { 1+ 1 }}
t idx-plus1 {string index abcdefgh +1}
t idx-007 {string index abcdefgh 007}
t idx-0b {string index abcdefgh 0b10}
t idx-0o {string index abcdefgh 0o3}
t idx-bad1 {string index abcdefgh 1+}
t idx-bad2 {string index abcdefgh 1+x}
t idx-bad3 {string index abcdefgh x+1}
t idx-bad4 {string index abcdefgh end+x}
t idx-bad5 {string index abcdefgh ++1}
t idx-bad6 {string index abcdefgh 1++1}
t idx-bad7 {string index abcdefgh 1.5}
t idx-bad8 {string index abcdefgh end1}
t idx-bad9 {string index abcdefgh "" }
t idx-empty-end {string index abcdefgh end-100}
t idx-huge {string index abcdefgh 99999999999999999999999}
t idx-hugechain {string index abcdefgh 9223372036854775807+1}
t idx-neghuge {string index abcdefgh -9223372036854775808+9223372036854775808}
t lindex-chain {lindex {a b c d} 1+1}
t lrange-chain {lrange {a b c d} 1+1 end-1}
t srange-chain {string range abcdef 1+1 3}
t listparse-bspace {llength {b\   }}
t listparse-bspace2 {lindex {b\   } 0}
t concat-multi {concat {a b} { c d } {e}}
t concat-quote {concat a {"x y"} c}
t concat-bs-dollar {concat {a\$b} c}
t concat-nl {concat "a\nb" c}
t concat-tab {concat "a\tb" c}
t concat-brace-open {concat a {b{c}}
t concat-empty-mid {concat a {} b}
t concat-list-brace-elem {concat [list a\ b] {c}}
t concat-nested {concat x [list [list a b] c] y}
