if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t eof-stdout {eof stdout}
t eof-stderr {eof stderr}
t cfg-eofchar-q-stdin {fconfigure stdin -eofchar}
t cfg-eofchar-q-all {lindex [fconfigure stdin] 5}
t cfg-eofchar-set {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -eofchar \x1a; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-eofchar-set2 {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -eofchar [list \x1a \x03]; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-eofchar-badlist {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -eofchar "x \{"} m]; close $f; list $r $m $::errorCode}
t cfg-eofchar-empty {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -eofchar {}; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-eofchar-list-empty {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -eofchar {{} {}}; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-eofchar-3 {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -eofchar {a b c}} m]; close $f; list $r $m $::errorCode}
t cfg-badopt {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -bogus} m]; close $f; list $r $m $::errorCode}
t cfg-eofchar-highlist {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -eofchar [list {} \x80]} m]; close $f; list $r $m $::errorCode}
