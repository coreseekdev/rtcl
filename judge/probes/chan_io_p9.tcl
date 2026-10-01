if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t full-cfg-file {set f [open /tmp/rtcl_probe.txt w]; set r [fconfigure $f]; close $f; set r}
t full-cfg-stdout {fconfigure stdout}
t eofchar-ab {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -eofchar ab} m]; close $f; list $r $m}
t eofchar-two-in {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -eofchar {ab cd}} m]; close $f; list $r $m}
t eofchar-rw {set f [open /tmp/rtcl_probe.txt w+]; fconfigure $f -eofchar [list \x1a \x03]; set r [fconfigure $f -eofchar]; close $f; set r}
t postevent {chan postevent stdout read}
t pop {chan pop stdout}
t truncate {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan truncate $f 0} m]; close $f; list $r $m}
t pipe {chan pipe}
t copy-bad {catch {chan copy stdout stdin} e; set e}
t copy-ok {set f [open /tmp/rtcl_probe.txt w]; set r [catch {chan copy stdout $f} m]; close $f; list $r $m}
t blocked {chan blocked stdin}
t event {chan event stdin {}}
t create2 {proc h {m args} {return {initialize finalize read write}}; set a [chan create {r} h]; set b [chan create {r} h]; list $a $b}
t push2 {proc h2 {m args} {return {initialize finalize read write}}; set ch [open /tmp/rtcl_probe.txt w]; set a [chan push $ch h2]; close $ch; set a}
t seek-stdin {seek stdin 0}
t tell-stdout {tell stdout}
