# dict getdef / getwithdefault (tclsh 9.1, TIP 342) — behavior pins.
puts [dict getdef {a b} a c]
puts [dict getdef {a b} b c]
puts [dict getdef {a {b c}} a b d]
puts [dict getdef {a {b c}} a c d]
puts [dict getdef {a {b c}} b c d]
catch {dict getdef {a {b c d}} a b d} m; puts "1:$m :: $errorCode"
catch {dict getdef} m; puts "2:$m"
catch {dict getdef {}} m; puts "3:$m"
catch {dict getdef {} {}} m; puts "4:$m"
catch {dict getdef {a b c} d e} m; puts "5:$m :: $errorCode"
puts [dict getwithdefault {a b} a c]
puts [dict getwithdefault {a b} z c]
catch {dict getwithdefault} m; puts "6:$m"
catch {dict getwithdefault {a b c} d e} m; puts "7:$m :: $errorCode"
catch {dict gorp} m; puts "8:$m :: $errorCode"
# exact match wins over prefix ambiguity
puts [dict get {a b} a]
puts [dict ge {a b} a]
catch {dict getd {a b} a c} m; puts "9:$m"
# default is returned verbatim (any value shape)
puts [dict getdef {a b} q {x y z}]
