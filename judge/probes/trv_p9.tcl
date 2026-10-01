proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# --- C3: what are the 10 vars visible during namespace delete? ---
proc seeVars {vtraced vidx op} {
    lappend ::seen [info vars ::ref::*]
}
catch {namespace delete ::ref}
namespace eval ::ref {}
set ::ref::var1 AAA
trace add variable ::ref::var1 unset seeVars
set ::seen {}
t nsdel-varnames {namespace delete ::ref}
t nsdel-seen {set ::seen}

# can an unset trace resurrect the variable?
proc resurrect {n i o} { set ::$n 77 }
catch {namespace delete ::rez}
namespace eval ::rez {}
set ::rez::v 1
trace add variable ::rez::v unset resurrect
t rez-del {namespace delete ::rez}
t rez-after {info exists ::rez::v}
t rez-val {catch {set ::rez::v}}

# --- D: 34.4 / 34.5 shapes ---
proc cb {old - -} { lappend ::which "$old -> [namespace which -command $old]" }
catch {namespace delete ::foo}
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete cb
set ::which {}
t del344 {namespace delete ::foo}
t del344-which {set ::which}

proc cb2 {old - -} { lappend ::which2 "$old -> [namespace which -command $old] [namespace current]" }
catch {namespace delete ::foo}
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete cb2
set ::which2 {}
t del345 {namespace eval ::foo namespace delete ::foo}
t del345-which {set ::which2}

# --- F: execution trace add on missing command ---
t exec-add-missing {catch {trace add execution nosuchcmd enter tc} m}
t exec-add-missing-msg {set m}
t exec-info-missing {catch {trace info execution nosuchcmd} m2}
t exec-info-missing-msg {set m2}

# --- G: proc redefinition fires delete-style rename trace? ---
set fired {}
proc trc {old new op} { lappend ::fired "$old|$new|$op" }
proc foo {} {}
trace add command foo rename trc
proc foo {} {puts x}
t redefine-fired {set ::fired}
t redefine-info {trace info command foo}
rename foo {}
set fired {}
proc foo {} {}
trace add command foo rename trc
proc foo {} {}
t redefine2-fired {set ::fired}

# does redefine preserve old traces? (fire check with live callback)
set fired {}
proc foo {} {}
trace add command foo rename trc
proc bar2 {} {}
trace add command bar2 rename trc
proc foo {} {}
proc bar2 {} {}
t redefine-fired3 {set ::fired}
t info-foo {trace info command foo}
