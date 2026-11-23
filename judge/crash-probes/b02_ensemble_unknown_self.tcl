# crash-probe: b02 ensemble unknown self
source [file join [file dirname [info script]] _chk.tcl]
namespace eval e2 {
    proc Magic {ens sub args} { return [list $ens nosuch2 {*}$args] }
    namespace ensemble create -unknown ::e2::Magic
}
chk rc [catch {e2 foo} e] 1
