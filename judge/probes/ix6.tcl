proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t cx-runinvalid {concat a "\{b"}
t cx-runinvalid2 {concat a "\{b c\}"}
t idx-010 {string index abcdefgh 010}
t idx-0o10 {string index abcdefgh 0o10}
t idx-00 {string index abcdefgh 00}
t idx-end010 {string index abcdefgh end+010}
t tr-evbs {concat {x\\ } 1}
t rp-x {string repeat ab x}
t rp-neg {string repeat ab -2}
t rp-hex {string repeat ab 0x3}
t rp-ws {string repeat ab { 3 }}
t rp-chain {string repeat ab 1+2}
t wi-err {string wordstart "a b c" x}
t wi-chain {string wordend "a b c" 0+0}
t fi-chain {string last c abcdef 5+0}
t sr-endneg {string range abcdef end-10 end}
