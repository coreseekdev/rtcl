if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t buf-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffering bogus} m]; close $f; list $r $m $::errorCode}
t trans-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -translation bogus} m]; close $f; list $r $m $::errorCode}
t block-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -blocking bogus} m]; close $f; list $r $m $::errorCode}
t size-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffersize bogus} m]; close $f; list $r $m $::errorCode}
t enc-bad {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -encoding bogus} m]; close $f; list $r $m $::errorCode}
t enc-bad2 {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -encoding} m]; close $f; list $r $m $::errorCode}
t odd-args {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffering} m]; close $f; list $r $m}
t odd-args2 {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffering line extra} m]; close $f; list $r $m}
t query-block {set f [open /tmp/rtcl_probe.txt w]; set r [fconfigure $f -blocking]; close $f; set r}
t q-translation-w {set f [open /tmp/rtcl_probe.txt w]; set r [fconfigure $f -translation]; close $f; set r}
t q-encoding {set f [open /tmp/rtcl_probe.txt w]; set r [fconfigure $f -encoding]; close $f; set r}
t puts-stdin {catch {puts stdin hi} m; set m}
t gets-dir {catch {gets stdout} m; set m}
t read-dir {catch {read stdout} m; set m}
t read-count-neg {catch {read stdin -1} m; set m}
t read-count-big {set f [open /tmp/rtcl_probe.txt r]; set r [read $f 1000000000000000000000]; close $f; list [string length $r]}
