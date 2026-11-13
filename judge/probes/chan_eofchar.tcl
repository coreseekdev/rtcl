# -eofchar read/write faces: the in-char stops reads at the byte level
# (read/gets/chan copy), sets EOF, and is swallowed; an empty line at the
# eofchar is EOF-shaped (gets -1). The out-char is written once at close.
set d /tmp
proc t {label script} {
    if {![info exists ::errorCode]} {set ::errorCode NONE}
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r|EC=$::errorCode"
}
set h [open $d/rtcl_eo1.txt w]; fconfigure $h -encoding binary; puts -nonewline $h "ab\x1acd\x1aef"; close $h
set h [open $d/rtcl_eo3.txt w]; fconfigure $h -encoding binary; puts -nonewline $h "x1\ny2\x1az3\nw4"; close $h
set h [open $d/rtcl_eo4.txt w]; fconfigure $h -encoding utf-8; puts -nonewline $h "héllo\x1awörld"; close $h
t read-eofchar {set f [open $d/rtcl_eo1.txt r]; fconfigure $f -encoding binary; fconfigure $f -eofchar \x1a; set s [read $f]; set e [eof $f]; close $f; list $s $e}
t readn-eofchar {set f [open $d/rtcl_eo1.txt r]; fconfigure $f -encoding binary; fconfigure $f -eofchar \x1a; set s [read $f 100]; set e [eof $f]; close $f; list [string length $s] $e}
t gets-eofchar {set f [open $d/rtcl_eo1.txt r]; fconfigure $f -encoding binary; fconfigure $f -eofchar \x1a; set a [gets $f v1]; set b [list $v1 [eof $f] [fblocked $f]]; set c [gets $f v2]; set e [eof $f]; close $f; list $a $b $c $e}
t midline {set f [open $d/rtcl_eo3.txt r]; fconfigure $f -encoding binary; fconfigure $f -eofchar \x1a; set r {}; while {[gets $f v] >= 0} {lappend r $v}; lappend r eof=[eof $f]; close $f; set r}
t utf8-eofchar {set f [open $d/rtcl_eo4.txt r]; fconfigure $f -eofchar \x1a; set s [read $f]; set e [eof $f]; close $f; list $s $e}
t out-eofchar {set f [open $d/rtcl_eo5.txt w]; fconfigure $f -eofchar [list {} \x03]; puts $f "l1"; puts $f "l2"; close $f; set f [open $d/rtcl_eo5.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; binary encode hex $s}
t out-flush-nochar {set f [open $d/rtcl_eo6.txt w]; fconfigure $f -eofchar [list {} \x03]; puts -nonewline $f "A"; flush $f; puts -nonewline $f "B"; close $f; set f [open $d/rtcl_eo6.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; binary encode hex $s}
t set-face {set f [open $d/rtcl_eo1.txt r]; fconfigure $f -eofchar [list \x1a \x03]; set q [fconfigure $f -eofchar]; set all [fconfigure $f]; close $f; list $q [string range $all 0 21]}
t bad-eofchar {set f [open $d/rtcl_eo1.txt r]; set c [catch {fconfigure $f -eofchar é} m]; set q [fconfigure $f -eofchar]; close $f; list $c $m $q}
t copy-eofchar {set a [open $d/rtcl_eo1.txt r]; fconfigure $a -encoding binary; fconfigure $a -eofchar \x1a; set b [open $d/rtcl_eo7.txt w]; fconfigure $b -encoding binary; set n [chan copy $a $b]; close $a; close $b; set f [open $d/rtcl_eo7.txt r]; fconfigure $f -encoding binary; set s [read $f]; close $f; list $n [binary encode hex $s]}
