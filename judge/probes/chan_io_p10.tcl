if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t ec-after-invalid {set ::errorCode FOO; catch {nosuchcmd} m; set ::errorCode}
t ec-after-plain {set ::errorCode FOO; catch {error msg} m; set ::errorCode}
t ec-after-success {set ::errorCode FOO; catch {set y 1} m; set ::errorCode}
t ec-int {set ::errorCode FOO; catch {eof 5x} m; set ::errorCode}
t eof-rw-file {set f [open /tmp/rtcl_probe.txt w+]; set r [eof $f]; close $f; set r}
t seek-file-r {set f [open /tmp/rtcl_probe.txt r]; set r [catch {seek $f 0} m]; close $f; list $r $m}
t flush-closed-err {set f [open /tmp/rtcl_probe.txt w]; close $f; catch {flush $f} m; set m}
t read-count-nonint {catch {read stdin x} m; set m}
t seek-neg {set f [open /tmp/rtcl_probe.txt r]; set r [catch {seek $f -1} m]; close $f; list $r $m}
