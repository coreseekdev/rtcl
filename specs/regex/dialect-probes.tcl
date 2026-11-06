# BRE/ERE 方言口径探针（oracle: tclsh 8.6.17）。
# 用法：tclsh dialect-probes.tcl > o.txt
#       RTCL_REGEX_ENGINE=spencer rtcl -f dialect-probes.tcl > r.txt
#       diff o.txt r.txt
set ob "\\{"
set cb "\\}"
proc t {pat str} {
    if {[catch {regexp -inline -- $pat $str} m]} {
        puts "P<$pat> S<$str> => ERR <$m>"
    } else {
        puts "P<$pat> S<$str> => <$m>"
    }
}
proc ta {pat} {
    if {[catch {regexp -about -- $pat} m]} {
        puts "A<$pat> => ERR <$m>"
    } else {
        puts "A<$pat> => <$m>"
    }
}

puts "== BRE operators =="
t {(?b)a|b} {a|b}
t {(?b)a|b} {a}
t {(?b)a\|b} {a|b}
t {(?b)\(a\)} {a}
t {(?b)(a)} {(a)}
t {(?b)a+} {a+}
t {(?b)a+} {aaa}
t {(?b)a\+} {aaa}
t {(?b)a?} {a?}
t {(?b)a\{2\}} {aa}
t {(?b)a\{2,\}} {aaaa}
t {(?b)a\{,2\}} {aa}
t {(?b)a\{2\}?} {aa?}
t {(?b)\(a\)\1} {aa}
t {(?b)*a*} {*aa}
t {(?b)\(*a\)} {*a}

puts "== BRE anchors =="
t {(?b)^a$} {a}
t {(?b)a^b} {a^b}
t {(?b)a$b} {a$b}
t {(?b)$a} {$a}
t {(?b)\(^a\)} {a}
t {(?b)\(a$\)} {a}

puts "== BRE escapes/classes =="
t {(?b)\d} {d}
t {(?b)\n} {n}
t {(?b)\A} {A}
t {(?b)\x41} {x41}
t {(?b)\\ } {\ }
t {(?b)\}} {x}
t {(?b)\0} {0}
t {(?b)\<} { a}
t {(?b)\>} {a }
t {(?b)[\d]} {d\n}
t {(?b)[\n]} {\n}
t {(?b)[a-c]*} {abc}

puts "== ERE operators/escapes =="
t {(?e)a|b} {a}
t {(?e)(a)} {a}
t {(?e)a+} {aaa}
t {(?e)a\{2\}} {aa}
t {(?e)(a)\1} {a1}
t {(?e)\d} {d}
t {(?e)\n} {n}
t {(?e)\A} {A}
t "(?e)$ob" "{"
t "(?e)$cb" "}"
t {(?e)[\d]} {d\n}
t {(?e)[\n]} {\n}
t {(?e)\<} {<}
t {(?e)(a))} {a)}
t {(?e))} {)}
t {(?e))(a)} {)a}

puts "== dialect switches =="
t {(?b)a(?e)b} {ab}
t {***:a|b} {a}
t {***:(?b)a|b} {a|b}
t {(?b)(?i)a} {a}

puts "== compile errors =="
t {(?b)a\{2} {aa}
t {(?b)a\{2,1\}} {aa}
t {(?b)a\{256\}} {aa}
t {(?b)\10} {x}
t "(?b)$ob" "x"
t {(?e)a+?} {a}
t {(?e)(?i)a} {a}
t {(?e)(?:a)} {a}
t {(?e)(?=a)} {a}
t {a(?i)b} {ab}
t {a(?-i)b} {ab}
t {a(?z)b} {ab}
t "a{2" "aa"
t "a{2x" "aa"

puts "== class ranges =="
t {[a-\d]} {a}
t {[\d-a]} {a}
t {[a-b-c]} {ab}
t {[a-b-]} {ab-}
t {[a-bc-d]} {abcd}
t {[--0]} {-0}
t {[0--]} {0}
t {[z-a} {z}
t {[z-ab} {z}
t {[z-a[} {z}
t {[a-b[:foo:]} {a}

puts "== -about flags =="
ta {abc}
ta {(?e))}
ta {(?e)(a))}
ta {(?b)a|b}
ta {(?e)a|b}
ta {a+?}
ta {\d}
ta {[[:alpha:]]}
ta {***=(a)}

puts "== \\A / -start / -all =="
t {\Aa} {ba}
puts "S1: [regexp -start 1 {\Aa} ba] [regexp -start 1 {^a} ba]"
puts "S2: [regexp -all -inline {(\A)|x} x]"
puts "S3: [regexp -all -inline {\A|a} aa]"
set x {}
lappend x [regsub -start 0 -all {\A(\w)} abcde {/\1}]
lappend x [regsub -start 2 -all {\A(\w)} abcde {/\1}]
puts "S4: $x"
