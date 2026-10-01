if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
set f [open /tmp/rtcl_probe2.txt w]; puts $f "ab\ncd"; close $f
t c-bogus {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -bogus} m]; close $src; close $g; list $r $m}
t c-extra {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g extra} m]; close $src; close $g; list $r $m}
t c-size-extra {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -size 1 extra} m]; close $src; close $g; list $r $m}
t c-size-bogus2 {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -size 1 -bogus 2} m]; close $src; close $g; list $r $m}
t c-size-noval {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -size} m]; close $src; close $g; list $r $m}
t c-size-badval {set src [open /tmp/rtcl_probe2.txt]; set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy $src $g -size x} m]; close $src; close $g; list $r $m}
t c-opt-first {set g [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy -size 2 a b} m]; close $g; list $r $m}
