proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# --- A: namespace eval {} ---
t ns-eval-empty-current {namespace eval {} {namespace current}}
t ns-eval-empty-vars {namespace eval {} {set q 1; info vars}}
t ns-eval-empty-which {namespace eval {} {set qq 2; namespace which -variable qq}}

# --- B: parent-ns errors ---
namespace eval pns {}
t set-into-missing {set ::nope::zz 1}
t set-trailing-missing {set ::nope2:: 1}
t set-trailing-ok {set ::pns:: 5}
t get-trailing {set ::pns::}
t var-missing-parent {namespace eval pns {variable sev:::en 7}}
t var-missing-parent-abs {variable ::absn::v 3}
t var-trailing-missing {namespace eval pns {variable deep:: 7}}
t var-ok-trailing {namespace eval pns {variable lst:: 7}; set pns::lst::}
t set-elem-missing-ns {set ::nope3::a(k) 1}

# --- C: unset traces + info vars during namespace delete ---
proc doTrace {vtraced vidx op} {
    global info
    append info [catch {set ::$vtraced}][llength [info vars ::ref::*]]
}
catch {namespace delete ::ref}
namespace eval ::ref {}
set ::ref::var1 AAA
trace add variable ::ref::var1 unset doTrace
set ::ref::var2 BBB
trace add variable ::ref::var2 {unset} doTrace
set info {}
t ns-delete-fires {namespace delete ::ref}
t ns-delete-info {set info}
t ns-delete-which {info exists ::ref::var1}

# what vars match ::ref::* inside an existing empty namespace?
namespace eval ::ref2 {}
t ref2-vars {info vars ::ref2::*}
t ref2-all {llength [info vars ::ref2::*]}

# --- C2: unset trace firing order on namespace delete; can trace resurrect? ---
proc ordr {n i o} { lappend ::ord "$n|$i|$o" }
catch {namespace delete ::ordns}
namespace eval ::ordns {}
set ::ordns::a 1
set ::ordns::b 2
trace add variable ::ordns::a unset ordr
trace add variable ::ordns::b unset ordr
set ::ord {}
t ordns-delete {namespace delete ::ordns}
t ordns-log {set ::ord}
