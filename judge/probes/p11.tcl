# 9.1 重钉（2026-11-28，T1 余项批）：8.6 时代 `3lt2` 是 invalid
# bareword；9.0 起词比较符 lt/le/gt/ge 与 eq/ne/in/ni 同走
# tclCompExpr.c ParseLexeme 的词内粘连规则（第 3 字节非 ASCII 字母即成
# 运算符）。钉形逐字节对照 tclsh9.1 冻结。
foreach e {3and2 3ne2 3lt2 3in2 3eq2x 3eqx 12eq1x 3or0 3eq2.0 3eq"2" 3ni2 3EQ2 3Lt2 3eq_x 3eq-2 3lté2 5ltinf 1.5lt2 0x10lt2 3lt2.5} {
    if {[catch {expr $e} m]} { puts "$e => ERR: [lindex [split $m \n] 0]" } else { puts "$e => '$m'" }
}
