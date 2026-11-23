# crash-probe: a04 format precision
source [file join [file dirname [info script]] _chk.tcl]
chk f [string length [format %.65536f 1.5]] 65538
chk e [string length [format %.70000e 1.5]] 70006
chk g [format %.99999999g 1.5] 1.5
chk star [string length [format %.*f 70000 1.5]] 70002
