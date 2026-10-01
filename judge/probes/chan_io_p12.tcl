if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t close-noargs {chan close}
t names-2args {chan names a b}
t pipe-arg {chan pipe x}
t copy-noargs {chan copy}
t truncate-noargs {chan truncate}
t pop-noargs {chan pop}
t postevent {chan postevent}
t blocked-noargs {chan blocked}
t eof-noargs {chan eof}
t create1 {chan create x}
t cfg-stdin-outchar {fconfigure stdin -eofchar {q q}}
t cfg-rw-one {set f [open /tmp/rtcl_probe.txt w+]; fconfigure $f -eofchar z; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-rw-half {set f [open /tmp/rtcl_probe.txt w+]; fconfigure $f -eofchar {z {}}; set r [fconfigure $f -eofchar]; close $f; set r}
t cfg-stdin-query {fconfigure stdin -eofchar}
t cfg-stdout-query {fconfigure stdout -eofchar}
t cfg-trans-platform {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -translation platform; set r [fconfigure $f -translation]; close $f; set r}
t cfg-trans-crlf {set f [open /tmp/rtcl_probe.txt w]; fconfigure $f -translation crlf; set r [fconfigure $f -translation]; close $f; set r}
t cfg-bad-bool {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -blocking x} m]; close $f; list $r $m $::errorCode}
t cfg-bad-size {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffersize x} m]; close $f; list $r $m $::errorCode}
t cfg-dangling-opt {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -buffering} m]; close $f; list $r $m}
t cfg-pfx {set f [open /tmp/rtcl_probe.txt w]; set r [fconfigure $f -buff]; close $f; set r}
t cfg-pfx-amb {set f [open /tmp/rtcl_probe.txt w]; set r [catch {fconfigure $f -b} m]; close $f; list $r $m}
t tell-file {set f [open /tmp/rtcl_probe.txt w]; puts -nonewline $f hello; set r [tell $f]; close $f; set r}
