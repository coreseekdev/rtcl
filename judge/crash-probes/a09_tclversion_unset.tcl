# crash-probe: a09 tclversion unset
source [file join [file dirname [info script]] _chk.tcl]
catch {unset ::tcl_version}
set r [catch {info tclversion} e]
chk rc $r 1
