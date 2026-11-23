# crash-probe: b01 ns unknown self
source [file join [file dirname [info script]] _chk.tcl]
namespace unknown ::nosuchhandler
set r [catch {nosuchcmd a b} e]
chk rc $r 1
namespace unknown {}
chk after $r 1
