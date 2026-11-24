# crash-probe: b09 json decode
source [file join [file dirname [info script]] _chk.tcl]
# Nesting is data: 30000 levels used to run the native stack out; now it
# is a clean error past the nesting limit, and documents within it decode.
set s [string repeat {[} 30000]
append s [string repeat {]} 30000]
set r [catch {json decode $s} e]
chk deep-rc $r 1
set s [string repeat {[} 1000]
append s [string repeat {]} 1000]
chk limit-rc [catch {json decode $s} e] 0
set v [lindex $s 0]
chk deep-schema [catch {json encode x [string repeat {list } 30000]} e] 1
