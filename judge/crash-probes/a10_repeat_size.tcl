# crash-probe: a10 repeat size
source [file join [file dirname [info script]] _chk.tcl]
chk sr1 [catch {string repeat ab 9223372036854775807} e] 1
chk lr1 [catch {lrepeat 9223372036854775807 x} e] 1
chk lr2 [catch {lrepeat 4611686018427387904 x} e] 1
chk sr2 [catch {string repeat a 1000000000000000} e] 1
