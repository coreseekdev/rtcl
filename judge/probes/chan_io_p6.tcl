proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t nocmd-names {catch {chan create {r w} foo} m; chan names}
t badbrace-ec {set p "foo \{"; catch {chan create {r w} $p} m; list $m $::errorCode}
t pend-file-in {set f [open /tmp/rtcl_probe.txt w]; puts $f hello; close $f; set f [open /tmp/rtcl_probe.txt r]; set r [chan pending input $f]; close $f; set r}
t pend-file-out {set f [open /tmp/rtcl_probe.txt w]; set r [chan pending output $f]; close $f; set r}
t pend-pipe-in {set p [open "|cat" r]; set r [chan pending input $p]; close $p; set r}
t pend-after-read {set f [open /tmp/rtcl_probe.txt r]; read $f 2; set r [chan pending input $f]; close $f; set r}
t pend-closed {catch {chan pending input file1000} m; list $m $::errorCode}
