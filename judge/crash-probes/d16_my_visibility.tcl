# crash-probe: d16 my visibility
# In tclsh `::oo::ObjN::my` is an ordinary command of the object's
# namespace: a qualified word names it from anywhere, a bare `my` only
# where that namespace is the current one.  A guard on the innermost frame
# rejected every qualified use (a method of another object, a plain proc).
source [file join [file dirname [info script]] _chk.tcl]
oo::class create TC { method go {n} { return "g$n" } }
TC create tc
set ns [info object namespace tc]

# a qualified `my` is the object's own command
chk qualified [catch {${ns}::my go 3} m] 0
chk qualified-val $m g3
proc pl {ns} { return [${ns}::my go 4] }
chk in-proc [catch {pl $ns} m] 0
chk in-proc-val $m g4
oo::class create Other { method m {n} { return "[${n}::my go 9]" } }
Other create o
chk other-object [catch {o m $ns} m] 0
chk other-object-val $m g9

# a bare `my` only where the object's namespace is current
chk ns-eval [catch {namespace eval $ns {my go 5}} m] 0
chk ns-eval-val $m g5
chk bare [catch {my go 6} m] 1
chk bare-msg $m {invalid command name "my"}
oo::class create Inner2 { method m {n} { return [namespace eval $n {my go 7}] } }
Inner2 create i2
chk ns-eval-in-method [catch {i2 m $ns} m] 0
chk ns-eval-in-method-val $m g7

# and gone with the object
chk destroyed [catch {tc destroy; ${ns}::my go 1} m] 1
chk destroyed-msg $m "invalid command name \"${ns}::my\""

# not visible outside: top level, another namespace, a plain proc, the
# global scope reached with uplevel
proc V {script} { set c [catch {uplevel 1 $script} m]; return "$c:$m" }
chk top [V { my x }] {1:invalid command name "my"}
chk other-ns [V {namespace eval ::n { my x }}] {1:invalid command name "my"}
proc plain {} { return [catch { my x } m] }
chk plain-proc [V {plain}] {0:1}
oo::class create A2 { method m2 {} { return [list [catch {uplevel #0 {my x}} m] $m] } }
A2 create a2
chk uplevel-global [a2 m2] {1 {invalid command name "my"}}
oo::class create A3 { method m3 {} { return [list [catch {namespace eval ::n {my x}} m] $m] } }
A3 create a3
chk ns-in-method [a3 m3] {1 {invalid command name "my"}}
