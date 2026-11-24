# crash-probe: d17 interp alias
# `interp alias {} name {} target ?prefix ...?` is forwarded to the alias
# table without the leading command word, so `alias` read the *target* as
# the new alias's name: the alias was created under the wrong name (and
# the target's own name shadowed when a prefix was given).
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE
proc tgt {a b} { return "tgt $a $b" }
chk create [catch {interp alias {} als {} tgt X} m] 0
chk create-name $m als
chk call [catch {als Y} m] 0
chk call-val $m {tgt X Y}
chk query [catch {interp alias {} als} m] 0
chk query-val $m {tgt X}
chk target-intact [catch {tgt Y Z} m] 0
chk target-intact-val $m {tgt Y Z}
chk listing [interp aliases] als
chk create2 [catch {interp alias {} als2 {} tgt} m] 0
chk create2-name $m als2
chk call2 [catch {als2 P Q} m] 0
chk call2-val $m {tgt P Q}
# the query of a name that is not an alias is empty, not an error
chk query-unknown [catch {interp alias {} nosuch} m] 0
chk query-unknown-val $m {}
# anything but {} as a path names an interpreter, and there is none
# (slavePath comes first, then slaveCmd, masterPath, masterCmd)
chk bad-slave [catch {interp alias other x {} y} m] 1
chk bad-slave-msg $m {could not find interpreter "other"}
chk bad-slave-ec $::errorCode {TCL LOOKUP INTERP other}
chk bad-master [catch {interp alias {} z other y} m] 1
chk bad-master-msg $m {could not find interpreter "other"}
chk too-few [catch {interp alias {} z} m] 0
chk no-args [catch {interp alias} m] 1
chk no-args-msg $m {wrong # args: should be "interp alias slavePath slaveCmd ?masterPath masterCmd? ?arg ...?"}

# the target is looked up like any command word, when the alias is called
namespace eval ns { proc tgt {a} { return "ns-tgt $a" } }
set c [catch {interp alias {} a1 {} ns::tgt} m]
chk qualified-define $c 0
chk qualified-call [catch {a1 5} m] 0
chk qualified-val $m {ns-tgt 5}
interp alias {} a3 {} ::tgt
chk absolute-call [catch {a3 1 2} m] 0
chk absolute-val $m {tgt 1 2}
set c [catch {interp alias {} c0 {} list; interp alias {} c1 {} c0; interp alias {} c2 {} c1; c2 x} m]
chk chain $c 0
chk chain-val $m x
set c [catch {interp alias {} a6 {} nosuch; a6} m]
chk missing-target $c 1
chk missing-target-msg $m {invalid command name "nosuch"}
# an error in the target carries the alias's frame
proc boom {} { error inside-boom }
interp alias {} ab {} boom
set c [catch {ab} m o]
chk err-code $c 1
chk err-msg $m inside-boom
chk err-ei [string map {"    " ""} [dict get $o -errorinfo]] {inside-boom
while executing
"error inside-boom "
(procedure "boom" line 1)
invoked from within
"ab"}
