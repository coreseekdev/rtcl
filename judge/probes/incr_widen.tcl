# incr_widen — 2026-11-16 收尾固化：incr 的 errorCode 面（坏旧值/
# 坏增量 = TCL VALUE INTEGER）与 i64 溢出 bignum 提升（正负双向，
# tclsh 升 2^63/-2^63-1 不回绕）。Agent 6 性能轮入账的两预存分歧。
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|$::errorCode" }
t badcur {set x abc; catch {incr x} e; list $e}
t badamt {set x 12; catch {incr x 3z} e; list $e}
t up {set y 9223372036854775807; incr y; set y}
t down {set z -9223372036854775808; incr z -1; set z}
t plain {set w 5; list [incr w] $w $::errorCode}
t bothbad {catch {incr qq 0x} e; list $e $::errorCode}
