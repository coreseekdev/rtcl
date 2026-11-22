proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# exact 18.4 shape but recording the var list
proc doTrace2 {vtraced vidx op} {
    global info
    append info "|$vtraced|[catch {set ::$vtraced}]|[info vars ::ref::*]|"
}
catch {namespace delete ::ref}
namespace eval ::ref {}
set ::ref::var1 AAA
trace add variable ::ref::var1 unset doTrace2
set ::ref::var2 BBB
trace add variable ::ref::var2 {unset} doTrace2
set info {}
t n184 {namespace delete ::ref}
t n184-info {set info}

# is it the failed `set ::ref::var1` that leaves 10 vars?  try without the set:
proc seeOnly {vtraced vidx op} {
    global info
    append info "|$vtraced|[info vars ::ref::*]|"
}
catch {namespace delete ::ref}
namespace eval ::ref {}
set ::ref::var1 AAA
trace add variable ::ref::var1 unset seeOnly
set info {}
t noSet {namespace delete ::ref}
t noSet-info {set info}

# just evaluate `set ::ref::var1` at top level after delete: does info vars grow?
catch {namespace delete ::ref}
namespace eval ::ref {}
set ::ref::var1 AAA
t pre {llength [info vars ::ref::*]}
catch {set ::ref::var1}
t post-failed-set {llength [info vars ::ref::*]}
t post-names {info vars ::ref::*}

# 344/345 shapes: namespace exists during command delete trace?
proc probeNs {old new op} {
    lappend ::pns "exists:[namespace exists ::foo] kids:[namespace children ::] which:[namespace which -command ::foo::bar]"
}
catch {namespace delete ::foo}
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete probeNs
set ::pns {}
t out-del {namespace delete ::foo}
t out-pns {set ::pns}

proc probeNs2 {old new op} {
    lappend ::pns2 "cur:[namespace current] exists:[namespace exists ::foo] kids:[namespace children ::] which:[namespace which -command ::foo::bar]"
}
catch {namespace delete ::foo}
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete probeNs2
set ::pns2 {}
t in-del {namespace eval ::foo namespace delete ::foo}
t in-pns {set ::pns2}

# variable unset traces: 344-shape for vars — which/exists during var unset trace
proc probeVar {n i o} {
    lappend ::pvs "exists:[namespace exists ::foov] vars:[info vars ::foov::*] which:[namespace which -variable ::foov::x]"
}
catch {namespace delete ::foov}
namespace eval ::foov {}
set ::foov::x 1
trace add variable ::foov::x unset probeVar
set ::pvs {}
t varout-del {namespace delete ::foov}
t varout-pvs {set ::pvs}
