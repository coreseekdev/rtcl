proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t f1 {
    namespace eval ::ref {}
    set ::ref::var1 AAA
    trace add variable ::ref::var1 unset doTrace
    set ::ref::var2 BBB
    trace add variable ::ref::var2 {unset} doTrace
    proc doTrace {vtraced vidx op} {
        global info
        append info [catch {set ::$vtraced}][llength [info vars ::ref::*]]
    }
    set info {}
    namespace delete ::ref
    rename doTrace {}
    set info
}
t f2 {
    variable x notrace
    proc callback {old - -} {
        variable x "$old exists: [namespace which -command $old]"
    }
    namespace eval ::foo {proc bar {} {}}
    trace add command ::foo::bar delete [namespace code callback]
    namespace delete ::foo
    set x
}
t f3 {
    variable x notrace
    proc callback {old - -} {
        variable x "$old exists: [namespace which -command $old]"
    }
    namespace eval ::foo {proc bar {} {}}
    trace add command ::foo::bar delete [namespace code callback]
    namespace eval ::foo namespace delete ::foo
    set x
}
