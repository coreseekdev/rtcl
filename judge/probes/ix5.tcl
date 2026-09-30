proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t nb-m4g {string index abcdefgh -4294967295}
t nb-m4gp1 {string index abcdefgh -4294967296}
t nb-mint {string index abcdefgh -2147483648}
t nb-mintm1 {string index abcdefgh -2147483649}
t so-sum {string index abcdefgh 4294967295+4294967295}
t so-endsum {string index abcdefgh end+4294967295}
t so-negsum {string index abcdefgh 4294967295-4294967295}
t tr-tab {concat "x\t" 1}
t tr-nl {concat "x\n" 1}
t tr-cr {concat "x\r" 1}
t tr-tab-mid {concat "a\tb" c}
t tr-lead-tab {concat "\ta" 1}
t tr-multi {concat  a  b  }
t tr-bs-nl {concat "a\\\n" 1}
t lstdb {list {$a{b}c}}
t lstdb2 {list "a\"b"}
t lstq {list {a[b}c}
t cx-empty {concat a {} b}
t cx-wsonly {concat a {   } b}
t cx-tabonly {concat a "\t" b}
t cx-bracearg {concat a {b{c}d} e}
