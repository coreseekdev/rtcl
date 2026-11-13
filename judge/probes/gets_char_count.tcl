# gets returns the CHARACTER count (Tcl_NumUtfChars), not UTF-8 bytes;
# read n counts characters and stops on char boundaries.
set d /tmp
proc t {label script} {
    if {![info exists ::errorCode]} {set ::errorCode NONE}
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r"
}
set h [open $d/rtcl_u8.txt w]; fconfigure $h -encoding utf-8; puts -nonewline $h "héllo wörld 中文abc"; close $h
set h [open $d/rtcl_line.txt w]; fconfigure $h -encoding utf-8; puts $h "héllo 中文"; close $h
t gets-len {set f [open $d/rtcl_line.txt r]; set n [gets $f v]; set l [string length $v]; close $f; list $n $l}
t gets-var {set f [open $d/rtcl_line.txt r]; gets $f v; close $f; set v}
t read3 {set f [open $d/rtcl_u8.txt r]; set s [read $f 3]; set n [string length $s]; close $f; list $s $n}
t read10 {set f [open $d/rtcl_u8.txt r]; set s [read $f 10]; set n [string length $s]; close $f; set n}
t read-all-twice {set f [open $d/rtcl_u8.txt r]; set a [string length [read $f]]; set b [string length [read $f]]; set e [eof $f]; close $f; list $a $b $e}
