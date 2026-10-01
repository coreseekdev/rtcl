proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
set f [open /tmp/rtcl_probe.txt w]
t buffering-bad {fconfigure $f -buffering bogus}
t trans-bad {fconfigure $f -translation bogus}
t eofchar-bad {fconfigure $f -eofchar {a b c}}
t eofchar-nul {fconfigure $f -eofchar "\0"}
t eofchar-nonascii {fconfigure $f -eofchar "é"}
t blocking-bad {fconfigure $f -blocking x}
t size-bad {fconfigure $f -buffersize x}
t size-neg {fconfigure $f -buffersize -5; fconfigure $f -buffersize}
t odd-trailing {fconfigure $f -buffering}
t enc-query {fconfigure $f -encoding}
t enc-set {fconfigure $f -encoding utf-8; fconfigure $f -encoding}
t blk-set {fconfigure $f -blocking off; fconfigure $f -blocking}
close $f
