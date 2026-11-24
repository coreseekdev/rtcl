# crash-probe: d18 TclOO errorCodes and the recursion-limit error
# TclOO lookup failures raise `TCL LOOKUP METHOD/OBJECT/CLASS <name>` and
# the interpreter's call-depth limit `TCL LIMIT STACK`; rtcl left the
# previous errorCode in place, and worded the limit error its own way.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE
proc S {script} { set c [catch {uplevel 1 $script} m]; return "$c|$m|$::errorCode" }
oo::class create Q { method m1 {} { return 1 }; method m2 {a} { return 2 } }
Q create q

# unknown method: the name as typed, for an object command and for `my`
chk meth [S {q nosuch}] {1|unknown method "nosuch": must be destroy, m1 or m2|TCL LOOKUP METHOD nosuch}
chk meth-prefix [S {q m}] {1|unknown method "m": must be destroy, m1 or m2|TCL LOOKUP METHOD m}
chk meth-qualified [S {q ::nosuch}] {1|unknown method "::nosuch": must be destroy, m1 or m2|TCL LOOKUP METHOD ::nosuch}
oo::class create R { method m {} { return [catch {my nosuch} m] } }
R create r
chk meth-my [S {r m}] {0|1|TCL LOOKUP METHOD nosuch}

# next / self / nextto outside a method
chk ctx-next [S {::oo::Helpers::next}] {1|next may only be called from inside a method|TCL OO CONTEXT_REQUIRED}
chk ctx-self [S {::oo::Helpers::self}] {1|self may only be called from inside a method|TCL OO CONTEXT_REQUIRED}
chk ctx-nextto [S {::oo::Helpers::nextto x}] {1|nextto may only be called from inside a method|TCL OO CONTEXT_REQUIRED}

# `X does not refer to an object` / `… a class`
chk ref-define [S {oo::define nosuchcls {}}] {1|nosuchcls does not refer to an object|TCL LOOKUP OBJECT nosuchcls}
chk ref-objdefine [S {oo::objdefine nosuchobj {}}] {1|nosuchobj does not refer to an object|TCL LOOKUP OBJECT nosuchobj}
chk ref-info [S {info object class nosuchobj}] {1|nosuchobj does not refer to an object|TCL LOOKUP OBJECT nosuchobj}
chk ref-copy [S {oo::copy nosuchsrc x}] {1|nosuchsrc does not refer to an object|TCL LOOKUP OBJECT nosuchsrc}
chk ref-mixin [S {oo::define Q { mixin nosuchcls }}] {1|nosuchcls does not refer to an object|TCL LOOKUP OBJECT nosuchcls}
chk ref-class [S {oo::define q { method z {} {} }}] {1|q does not refer to a class|TCL LOOKUP CLASS q}

# other TclOO errors that carry a code
Q create x
chk overwrite [S {oo::copy Q x}] {1|can't create object "x": command already exists with that name|TCL OO OVERWRITE_OBJECT}
chk superclass [S {oo::class create D { superclass q }}] {1|only a class can be a superclass|TCL LOOKUP CLASS q}
chk mixin [S {oo::define Q { mixin q }}] {1|may only mix in classes|TCL LOOKUP CLASS q}

# the call-depth limit: tclsh's message and code
proc ra {} { ra }
chk limit [S {ra}] {1|too many nested evaluations (infinite loop?)|TCL LIMIT STACK}
