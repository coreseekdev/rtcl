# Source of judge/crash-probes/d22_string_map.tcl: `string map`, check by
# check.  `_gen.tcl` runs every line under tclsh and writes the probe with
# tclsh's outputs as the expected values.
#
# Timings are not among the checks (the probe pins results, and the
# results did not change); the speed is in the probe's head.
T first-listed {string map {a b ab cd} ab}
T first-listed2 {string map {ab cd a b} ab}
T equal-length-order {string map {ab X ab Y} ab}
T empty-source {string map {{} x a b} a}
T empty-input {string map {a b} {}}
T empty-map {string map {{}} abc}
T nocase {string map -nocase {AB x} aabAB}
T nocase-order {string map -nocase {a X aa Y} AAa}
T nocase-turkish {string map -nocase {i x} İI}
T nocase-ligature {string map -nocase {ß x} ẞ}
T nocase-multi {string map -nocase {abc X} aBcABCabcd}
T nocase-dst-case {string map -nocase {abc Xy} ABC abc}
T unicode {string map {é e} déjà éÉ}
T unicode-3 {string map {日 本} 日本日}
T unicode-4 {string map {😀 x} a😀b😀}
T unicode-nocase {string map -nocase {É x} éÉ}
T bad-option {string map -x {a b} aaa}
T two-args {string map -nocase {a b}}
T odd-map {string map {a} x}
T odd-map2 {string map {a b c} x}
T odd-map3 {string map {} x}
T not-a-list [list string map "a \\{b" x]
T empty-everything {string map {} {}}
T long-source [list string map [list [string repeat a 100] X] [string repeat a 250]]
T many-pairs {string map {a 1 b 2 c 3 d 4 e 5 f 6 g 7 h 8} abcdefgh}
T overlapping {string map {abc X ab Y} aabcab}
T chain {string map {a b b c} abab}
T big-input [list string map {a b b c} [string repeat ab 20000]]
puts DONE
