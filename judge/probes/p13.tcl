# 9.1 重钉（2026-11-28，T1 余项批）：3gt2/3le2/3ge2 在 8.6 是 invalid
# bareword，9.0+ 是词比较符粘连；末两行钉 PREC_COMPARE 优先级
# （`lt` 高于 `==`：5 == 10 lt 2 是 5 == ("10" lt "2") → 0）。
foreach e {3gt2 3le2 3ge2 3eq2 3ne2 3in2 {5 == 10 lt 2} {1 lt 2 == 1} {2 ge 10} {"2" gt "10"} {"abc" lt "abd"} {"a" lt "b"}} {
    if {[catch {expr $e} m]} { puts "$e => ERR: [lindex [split $m \n] 0]" } else { puts "$e => '$m'" }
}
