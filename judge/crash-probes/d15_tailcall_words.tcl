# crash-probe: d15 tailcall to a non-proc
# A `tailcall` naming a built-in (an ensemble, an alias, a method helper):
# tclsh looks the command up in the namespace the tailcall ran in and runs
# the words as written once the frame is gone (TclNRTailcallEval's
# lookupNsPtr), and the dispatcher logs the command (`TEOV_Error`).  The
# words used to be replaced by the resolved key and run without a log
# event; `tailcall my …` in a method failed outright.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE

# `bad name script want ?ec?`: the script fails with the first lines of
# its errorInfo (indentation dropped) equal to `want`, and the errorCode.
proc bad {name script want {ec NONE}} {
    set c [catch {uplevel 1 $script} m o]
    set ei [join [lrange [split [string map {"    " ""} [dict get $o -errorinfo]] \n]             0 [expr {[llength [split $want |]] - 1}]] |]
    uplevel 1 [list chk "$name-code" $c 1]
    uplevel 1 [list chk "$name-ei" $ei $want]
    uplevel 1 [list chk "$name-ec" $::errorCode $ec]
}

# the log event of the tailcalled command: `while executing <words>`
proc p1 {} { tailcall error x3 }
bad a {p1} {x3|while executing|"error x3"|invoked from within|"p1"}
proc p2 {} { tailcall set }
bad b {p2} {wrong # args: should be "set varName ?newValue?"|while executing|"set"|invoked from within|"p2"} {TCL WRONGARGS}
proc p4 {} { tailcall nosuch a b }
bad d {p4} {invalid command name "nosuch"|while executing|"nosuch a b"|invoked from within|"p4"} {TCL LOOKUP COMMAND nosuch}

# once something deeper has logged, the command's frame is `invoked from within`
proc p6 {} { tailcall foreach x {1} { error inner6 } }
bad f {p6} {inner6|while executing|"error inner6 "|("foreach" body line 1)|invoked from within|"foreach x 1 { error inner6 }"|invoked from within|"p6"}

# a raise site that wrote the errorInfo suppresses the frame
proc p5 {} { tailcall error "x5" "custom info" }
bad e {p5} {custom info|invoked from within|"p5"}

# an expr runtime error: first frame; a parse error: appended
proc p8 {} { tailcall expr {1/0} }
bad h {p8} {divide by zero|while executing|"expr 1/0"|invoked from within|"p8"} {ARITH DIVZERO {divide by zero}}
proc p7 {} { tailcall expr {1 +} }
bad g {p7} {missing operand at _@_|in expression "1 +_@_"|(parsing expression "1 +")|invoked from within|"expr {1 +}"|invoked from within|"p7"} {TCL PARSE EXPR MISSING}

# lookup in the frame's namespace, execution in the caller's
catch {rename ::a::myset set}
namespace eval a {}
rename set ::a::myset
namespace eval a { proc q {} { tailcall myset ::gv 1 } }
chk ns-lookup [catch {a::q} m] 0
chk ns-result $m 1
chk ns-global $::gv 1
rename ::a::myset set
namespace eval a { proc q2 {} { tailcall namespace current } }
chk ns-run [a::q2] ::
chk ns-after [catch {myset x} m] 1

# the words are values: never re-parsed as a script
proc w1 {} { tailcall set ::wv {$a [b] "c} }
w1
chk words-value $::wv {$a [b] "c}
proc w2 {} { tailcall list {$a} {b c} }
chk words-list [w2] {{$a} {b c}}

# the tailcalled command stands where the frame stood: one level
proc lvl {args} { return "[info level 0]|[info level]|[catch {info level -1} u]:$u" }
proc w3 {} { tailcall lvl a b }
chk level0 [w3] {lvl a b|1|1:bad level "-1"}

# ensembles, import aliases, object commands
namespace eval ens { proc e {args} { return "e:$args" }; namespace export e; namespace ensemble create }
proc w4 {} { tailcall ens e 1 }
chk ensemble [w4] {e:1}
namespace eval src { proc imp {x} { return "imp:$x [namespace current]" }; namespace export imp }
namespace eval dst { namespace import ::src::imp; proc w5 {} { tailcall imp 7 } }
chk import [dst::w5] {imp:7 ::src}
oo::class create P { method a {} { tailcall pp go 1 0 }; method go {x y} { return "go$x L=[info level]" } }
P create pp
chk object-cmd [catch {pp a} m] 0
chk object-result $m {go1 L=1}

# `my` in a method: the object's own command, found in the object's namespace
oo::class create TC { method go {n acc} {
    if {$n <= 0} { return "[info level]|[catch {info level -1} u]:$u" }
    tailcall my go [expr {$n - 1}] [expr {$acc + $n}]
} }
TC create tc
chk my [catch {tc go 100 0} m] 0
chk my-level $m {1|1:bad level "-1"}
oo::class create Q { method m {} { tailcall my } }
Q create q
bad my-args {q m} {wrong # args: should be "my method ?arg ...?"|while executing|"my"|invoked from within|"q m"} {TCL WRONGARGS}

# `next`/`self` no longer stand in a method
oo::class create B { method m {n} { return "b$n" } }
oo::class create D { superclass B; method m {n} { tailcall next $n } }
D create d
bad next {d m 2} {next may only be called from inside a method|while executing|"next 2"|invoked from within|"d m 2"} {TCL OO CONTEXT_REQUIRED}
oo::class create S { method m {} { tailcall self } }
S create s
bad self {s m} {self may only be called from inside a method|while executing|"self"|invoked from within|"s m"} {TCL OO CONTEXT_REQUIRED}
