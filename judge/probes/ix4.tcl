proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t ov-4g {string index abcdefgh 4294967296}
t ov-4gm {string index abcdefgh 4294967295}
t ov-1e12 {string index abcdefgh 1000000000000}
t ov-i64m1 {string index abcdefgh 9223372036854775806}
t ov-i64h {string index abcdefgh 9223372036854775807}
t ov-i64h2 {string index abcdefgh 4611686018427387904+4611686018427387903}
t ov-chain-intmin {string index abcdefgh -9223372036854775808}
t ov-end-i64h {string index abcdefgh end-9223372036854775808}
t cx-invalid "concat a \{b\{c"
t cx-invalid2 "concat a \[b"
t cx-invalid3 {concat a {b c}d}
t cx-quote-elem {concat a {"x y"} c}
t cx-dollar {concat {a$b} c}
t cx-brk {concat {a[b} c}
t cx-semi {concat {a;b} c}
t cx-backsl {concat {a\b} c}
t cx-hash-mid {concat {a#b} c}
t cx-nested-list {concat {a {b c}} d}
t cx-trail-bs {concat a {b\\}}
t lst-tab {list "a\tb"}
t lst-nl {list "a\nb"}
t lst-cr {list "a\rb"}
t lst-semi {list {a;b}}
t lst-dollar-brace {list {$a{b}}
t concat-vs-list {concat [list "a\tb" "a\nb"] {x}}
