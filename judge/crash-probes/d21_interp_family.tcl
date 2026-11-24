# crash-probe: d21 the `interp` command family
# Every tclsh form of `interp` was a `wrong # args` or "not supported";
# a child interpreter came up with every command its parent has (`exec`
# included), so `interp create -safe` made nothing safe; `interp hide`
# only took the name out of `info commands`; aliases into the master were
# misaddressed and a loop of aliases across interpreters had no end.
#
# Expected values are tclsh 8.6.17's own outputs, check by check, in the
# order the state is built: _gen_d21.tcl writes this file from
# _d21_source.tcl.  Where rtcl deliberately differs from tclsh the check
# is left out (the generator's SKIP list names each):
#   - rtcl sorts `interp slaves` / `interp hidden` (tclsh: hash order);
#   - tclsh's `unknown` handler recurses to the limit once `set` is
#     hidden in a child; rtcl says `invalid command name "set"`;
#   - rtcl deletes no interpreter that is, or holds one that is, being
#     evaluated;
#   - rtcl's own command set: `auto_*`, `job`, `wait` hidden in a safe
#     interpreter, `::tcl::mathfunc::min`/`max` built in, not aliases.
source [file join [file dirname [info script]] _chk.tcl]
proc T {script} {
    # A success leaves the previous error's errorCode standing: start
    # each check from NONE.
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    return [list $c $r [expr {[info exists ::errorCode] ? $::errorCode : {}}]]
}
# A skipped check still builds the state the later ones sit on: its
# script runs, unchecked.
proc U {script} {
    catch {uplevel 1 $script}
}

set ::errorCode NONE
# The helper the checks inside a child go through (defined where it
# is first used): code, result, the child's errorCode, its errorInfo
# with newlines as ^.

chk p-create [T {interp create}] {0 interp0 NONE}
chk p-create2 [T {interp create}] {0 interp1 NONE}
chk p-exists [T {list [interp exists] [interp exists interp0] [interp exists interp1] [interp exists nope] [interp exists {}]}] {0 {1 1 1 0 1} NONE}
U {lsort [interp slaves]}
chk p-nested-create [T {interp create {interp0 g}}] {0 {interp0 g} NONE}
chk p-nested-exists [T {list [interp exists {interp0 g}] [interp exists g] [interp exists interp0]}] {0 {1 0 1} NONE}
chk p-nested-slaves [T {lsort [interp slaves interp0]}] {0 g NONE}
chk p-nested-eval [T {interp eval {interp0 g} {set x 1}}] {0 1 NONE}
chk p-nested-path-cmd [T {interp eval interp0 {g eval {expr 2 * 3}}}] {0 6 NONE}
chk p-badpath [T {interp eval nope {set x 1}}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk p-badpath2 [T {interp eval {interp0 nope} {set x 1}}] {1 {could not find interpreter "interp0 nope"} {TCL LOOKUP INTERP {interp0 nope}}}
chk p-self [T {interp eval {} {expr 1 + 1}}] {0 2 NONE}
chk p-self-issafe [T {interp issafe}] {0 0 NONE}
chk p-issafe-nested [T {list [interp issafe interp0] [interp issafe {interp0 g}] [interp issafe nope]}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk p-ns-name [T {interp create ::a::b::c}] {0 ::a::b::c NONE}
chk p-ns-exists [T {list [namespace exists ::a] [namespace exists ::a::b] [interp exists ::a::b::c]}] {0 {1 1 1} NONE}
chk p-ns-eval [T {::a::b::c eval {expr 4 + 4}}] {0 8 NONE}
chk p-ns-delete [T {interp delete ::a::b::c}] {0 {} NONE}
chk p-ns-after [T {list [namespace exists ::a] [interp exists ::a::b::c]}] {0 {1 0} NONE}
chk p-children [T {lsort [interp children interp0]}] {0 g NONE}
chk p-aliases-nested [T {lsort [interp aliases {interp0 g}]}] {0 {} NONE}
chk p-hidden-nested [T {interp hidden {interp0 g}}] {0 {} NONE}
chk p-reclimit-nested [T {interp recursionlimit {interp0 g}}] {0 1000 NONE}
chk c-opt-after [T {interp create sa -safe}] {0 sa NONE}
chk c-opt-issafe [T {interp issafe sa}] {0 1 NONE}
chk c-opt-twice [T {interp create sb -safe -safe}] {0 sb NONE}
chk c-opt-dashdash [T {interp create -- sc}] {0 sc NONE}
chk c-opt-dashdash-name [T {interp create -- -sd}] {0 -sd NONE}
chk c-opt-bad [T {interp create -untrusted se}] {1 {bad option "-untrusted": must be -safe or --} {TCL LOOKUP INDEX option -untrusted}}
chk c-opt-two [T {interp create x y}] {1 {wrong # args: should be "interp create ?-safe? ?--? ?path?"} {TCL WRONGARGS}}
chk c-opt-existing [T {interp create sa}] {1 {interpreter named "sa" already exists, cannot create} NONE}
chk c-opt-existing-cmd [T {proc mycmd {} {return 1}; interp create mycmd}] {0 mycmd NONE}
chk c-opt-over-proc [T {info procs mycmd}] {0 {} NONE}
chk c-default-after-cmd [T {proc interp9 {} {}; interp create}] {0 interp2 NONE}
chk c-delete-both [T {interp delete sb sc -sd mycmd interp9}] {1 {could not find interpreter "interp9"} {TCL LOOKUP INTERP interp9}}
chk c-bare [T interp] {1 {wrong # args: should be "interp cmd ?arg ...?"} {TCL WRONGARGS}}
chk c-ev [T {interp ev sa {expr 5}}] {0 5 NONE}
chk c-e [T {interp e}] {1 {ambiguous option "e": must be alias, aliases, bgerror, cancel, children, create, debug, delete, eval, exists, expose, hide, hidden, issafe, invokehidden, limit, marktrusted, recursionlimit, slaves, share, target, or transfer} {TCL LOOKUP INDEX option e}}
chk c-al [T {interp al}] {1 {ambiguous option "al": must be alias, aliases, bgerror, cancel, children, create, debug, delete, eval, exists, expose, hide, hidden, issafe, invokehidden, limit, marktrusted, recursionlimit, slaves, share, target, or transfer} {TCL LOOKUP INDEX option al}}
chk c-bogus [T {interp bogus}] {1 {bad option "bogus": must be alias, aliases, bgerror, cancel, children, create, debug, delete, eval, exists, expose, hide, hidden, issafe, invokehidden, limit, marktrusted, recursionlimit, slaves, share, target, or transfer} {TCL LOOKUP INDEX option bogus}}
interp delete sa
chk d-self [T {interp delete {}}] {1 {cannot delete the current interpreter} {TCL OPERATION INTERP DELETESELF}}
chk d-missing [T {interp delete nope}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk d-none [T {interp delete}] {0 {} NONE}
chk d-multi [T {interp create d1; interp create d2; interp create d3; interp delete d1 nope d3}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk d-multi-after [T {list [interp exists d1] [interp exists d2] [interp exists d3]}] {0 {0 1 1} NONE}
chk d-cmd-gone [T {catch {d1 eval x} m; set m}] {0 {invalid command name "d1"} {TCL LOOKUP COMMAND d1}}
chk d-nested [T {interp create d4; interp eval d4 {interp create e}; interp delete {d4 e}}] {0 {} NONE}
chk d-nested-after [T {list [interp exists {d4 e}] [lsort [interp slaves d4]]}] {0 {0 {}} NONE}
chk d-outer [T {interp delete d4}] {0 {} NONE}
chk d-rename-away [T {interp create d5; interp alias d5 w {} list; rename d5 {}}] {0 {} NONE}
chk d-rename-after [T {list [interp exists d5] [info commands d5]}] {0 {0 {}} NONE}
U {interp create d6; interp alias d6 w {} interp delete d6; interp eval d6 w}
U {interp exists d6}
interp create h1
chk h-hide [T {interp hide h1 exec}] {0 {} NONE}
U {lsort [interp hidden h1]}
chk h-call [T {interp eval h1 {catch {exec echo hi} m; set m}}] {0 {invalid command name "exec"} NONE}
chk h-call-ec [T {interp eval h1 {catch {exec echo hi} m o; dict get $o -errorcode}}] {0 {TCL LOOKUP COMMAND exec} NONE}
chk h-info [T {interp eval h1 {info commands ex*}}] {0 {expr exit} NONE}
chk h-twice [T {interp hide h1 exec}] {1 {unknown command "exec"} {TCL LOOKUP COMMAND exec}}
chk h-twice-ec [T {catch {interp hide h1 exec} m o; dict get $o -errorcode}] {0 {TCL LOOKUP COMMAND exec} {TCL LOOKUP COMMAND exec}}
chk h-nosuch [T {interp hide h1 nosuchcmd}] {1 {unknown command "nosuchcmd"} {TCL LOOKUP COMMAND nosuchcmd}}
chk h-nosuchpath [T {interp hide nope exec}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk h-empty [T {interp hide h1 {}}] {1 {unknown command ""} {TCL LOOKUP COMMAND {}}}
chk h-invoke [T {interp invokehidden h1 exec echo hi}] {0 hi NONE}
chk h-invoke-hidden-name [T {interp eval h1 {catch {interp invokehidden h1 exec echo x} m; set m}}] {0 {could not find interpreter "h1"} NONE}
chk h-invoke-bad [T {interp invokehidden h1 nosuch cmd}] {1 {invalid hidden command name "nosuch"} {TCL LOOKUP HIDDENTOKEN nosuch}}
chk h-invoke-arity [T {interp invokehidden h1 exec}] {1 {wrong # args: should be "exec ?-option ...? arg ?arg ...?"} {TCL WRONGARGS}}
chk h-invoke-wa [T {catch {interp invokehidden h1 exec} m o; list $m [dict get $o -errorcode]}] {0 {{wrong # args: should be "exec ?-option ...? arg ?arg ...?"} {TCL WRONGARGS}} {TCL WRONGARGS}}
chk h-invoke-hidden-set [T {interp hide h1 set; interp invokehidden h1 set zz 3}] {0 3 NONE}
U {interp eval h1 {catch {set a 1} m; set m}}
U {interp eval h1 {catch {set a 1} m o; dict get $o -errorcode}}
chk h-hidden-info [T {interp eval h1 {info commands set}}] {0 {} NONE}
chk h-expose [T {interp expose h1 set}] {0 {} NONE}
chk h-expose-again [T {interp expose h1 set}] {1 {unknown hidden command "set"} {TCL LOOKUP HIDDENTOKEN set}}
chk h-expose-existing [T {interp hide h1 exec; interp expose h1 exec puts}] {1 {unknown command "exec"} {TCL LOOKUP COMMAND exec}}
chk h-expose-existing-ec [T {catch {interp expose h1 exec puts} m o; dict get $o -errorcode}] {0 {TCL EXPOSE COMMAND_EXISTS} {TCL EXPOSE COMMAND_EXISTS}}
chk h-expose-new [T {interp expose h1 exec myexec}] {0 {} NONE}
chk h-expose-new-call [T {interp eval h1 {myexec echo via-newname}}] {0 via-newname NONE}
chk h-expose-new-wa [T {interp eval h1 {catch {myexec} m o; dict get $o -errorinfo}}] {0 {wrong # args: should be "myexec ?-option ...? arg ?arg ...?"
    while executing
"myexec"} NONE}
U {lsort [interp hidden h1]}
chk h-expose-ns [T {interp hide h1 clock; interp expose h1 clock ::x}] {1 {cannot expose to a namespace (use expose to toplevel, then rename)} {TCL EXPOSE NON_GLOBAL}}
chk h-expose-ns-ec [T {catch {interp expose h1 clock ::x} m o; dict get $o -errorcode}] {0 {TCL EXPOSE NON_GLOBAL} {TCL EXPOSE NON_GLOBAL}}
chk h-hide-token-ns [T {interp hide h1 clock a::b}] {1 {cannot use namespace qualifiers in hidden command token (rename)} {TCL VALUE HIDDENTOKEN}}
chk h-hide-token [T {interp hide h1 clock ck}] {1 {unknown command "clock"} {TCL LOOKUP COMMAND clock}}
U {lsort [interp hidden h1]}
chk h-invoke-token [T {interp invokehidden h1 ck format 0 -format %Y}] {1 {invalid hidden command name "ck"} {TCL LOOKUP HIDDENTOKEN ck}}
chk h-hide-token-taken [T {interp hide h1 list lk}] {0 {} NONE}
chk h-hide-token-taken-ec [T {catch {interp hide h1 list lk} m o; dict get $o -errorcode}] {0 {TCL LOOKUP COMMAND list} {TCL LOOKUP COMMAND list}}
chk h-invoke-ns [T {interp invokehidden h1 -namespace ::nos ck format 0 -format %Y}] {1 {invalid hidden command name "ck"} {TCL LOOKUP HIDDENTOKEN ck}}
chk h-invoke-global [T {interp invokehidden -global h1 ck format 0 -format %Y}] {1 {could not find interpreter "-global"} {TCL LOOKUP INTERP -global}}
chk h-invoke-dashdash [T {interp invokehidden -- h1 ck format 0 -format %Y}] {1 {could not find interpreter "--"} {TCL LOOKUP INTERP --}}
chk h-invoke-badopt [T {interp invokehidden h1 -bogus x}] {1 {bad option "-bogus": must be -global, -namespace, or --} {TCL LOOKUP INDEX option -bogus}}
chk h-invoke-badopt2 [T {interp invokehidden h1 -bogus}] {1 {bad option "-bogus": must be -global, -namespace, or --} {TCL LOOKUP INDEX option -bogus}}
chk h-invoke-badopt-ec [T {catch {interp invokehidden h1 -bogus} m o; dict get $o -errorcode}] {0 {TCL LOOKUP INDEX option -bogus} {TCL LOOKUP INDEX option -bogus}}
chk h-invoke-ns-missing [T {interp invokehidden h1 -namespace}] {1 {wrong # args: should be "interp invokehidden path ?-namespace ns? ?-global? ?--? cmd ?arg ..?"} {TCL WRONGARGS}}
chk h-proc [T {interp eval h1 {proc hp {x} {return "hp:$x"}}; interp hide h1 hp}] {0 {} NONE}
chk h-invoke-proc [T {interp invokehidden h1 hp 7}] {0 hp:7 NONE}
chk h-invoke-proc-wa [T {catch {interp invokehidden h1 hp} m o; set m}] {0 {wrong # args: should be "hp x"} {TCL WRONGARGS}}
chk h-expose-proc [T {interp expose h1 hp; interp eval h1 {hp 8}}] {0 hp:8 NONE}
chk h-ensemble [T {interp hide h1 info; interp invokehidden h1 info commands set}] {0 set NONE}
U {interp hide h1 interp; interp eval h1 {catch {interp aliases} m; set m}}
U {interp invokehidden h1 interp hidden}
chk h-expose-interp [T {interp invokehidden h1 interp expose h1 interp}] {1 {could not find interpreter "h1"} {TCL LOOKUP INTERP h1}}
chk h-rename-hidden [T {interp eval h1 {catch {rename ck other} m; set m}}] {0 {can't rename "ck": command doesn't exist} NONE}
chk h-compiled-before [T {interp eval h1 {proc hs {} {set s 1; return $s}}; interp eval h1 hs}] {0 1 NONE}
chk h-hide-set [T {interp hide h1 set}] {0 {} NONE}
U {interp eval h1 {catch {hs} m o; set m}}
chk h-compiled-new [T {interp eval h1 {proc hs2 {} {set s 2; return $s}}; catch {hs2} m o; set m}] {0 {invalid command name "hs2"} {TCL LOOKUP COMMAND hs2}}
chk h-expose-set [T {interp expose h1 set}] {0 {} NONE}
chk h-compiled-back [T {interp eval h1 hs}] {0 1 NONE}
chk h-compiled-back2 [T {interp eval h1 hs2}] {0 2 NONE}
chk h-expose-list [T {interp expose h1 list lk}] {1 {unknown hidden command "list"} {TCL LOOKUP HIDDENTOKEN list}}
interp delete h1
interp create sv -safe
chk s-issafe [T {interp issafe sv}] {0 1 NONE}
U {lsort [lmap c [interp hidden sv] {expr {[string match tcl:* $c] ? "" : $c}}]}
U {lsort [interp aliases sv]}
chk s-exec [T {interp eval sv {catch {exec echo hi} m; set m}}] {0 {invalid command name "exec"} NONE}
U {interp eval sv {catch {open /etc/hostname} m; set m}}
U {interp eval sv {catch {file exists /etc/hostname} m; set m}}
chk s-cd [T {interp eval sv {catch {cd /} m; set m}}] {0 {invalid command name "cd"} NONE}
chk s-puts [T {interp eval sv {catch {puts hi} m; set m}}] {0 {can not find channel named "stdout"} NONE}
chk s-expr [T {interp eval sv {expr 1 + 1}}] {0 2 NONE}
U {interp eval sv {catch {source /etc/hostname} m; set m}}
chk s-set [T {interp eval sv {set x 1}}] {0 1 NONE}
chk s-glob [T {interp eval sv {catch {glob /*} m; set m}}] {0 {invalid command name "glob"} NONE}
chk s-env [T {interp eval sv {catch {array names env} m; set m}}] {0 {} NONE}
chk s-env2 [T {interp eval sv {catch {set env(PATH)} m; set m}}] {0 {can't read "env(PATH)": no such variable} NONE}
chk s-exit [T {interp eval sv {catch {exit} m o; dict get $o -errorcode}}] {0 {TCL LOOKUP COMMAND exit} NONE}
chk s-load [T {interp eval sv {catch {load x} m; set m}}] {0 {invalid command name "load"} NONE}
U {interp eval sv {catch {socket 127.0.0.1 1} m; set m}}
chk s-pwd [T {interp eval sv {catch {pwd} m; set m}}] {0 {invalid command name "pwd"} NONE}
chk s-fconfig [T {interp eval sv {catch {fconfigure stdin} m; set m}}] {0 {invalid command name "fconfigure"} NONE}
chk s-version [T {interp eval sv {info tclversion}}] {0 8.6 NONE}
chk s-platform [T {lsort [interp eval sv {array names tcl_platform}]}] {0 {byteOrder engine pathSeparator platform pointerSize threaded wordSize} NONE}
chk s-lib [T {interp eval sv {list [info exists tcl_library] [info exists tcl_pkgPath] [info exists env]}}] {0 {0 0 0} NONE}
chk s-clock [T {interp eval sv {clock format 0 -format %Y}}] {0 1970 NONE}
chk s-math [T {interp eval sv {list [::tcl::mathfunc::min 2 9] [::tcl::mathfunc::max 2 9]}}] {0 {2 9} NONE}
chk s-interp-exists [T {interp eval sv {info commands interp}}] {0 interp NONE}
chk s-create-child [T {interp eval sv {interp create k}}] {0 k NONE}
chk s-child-safe [T {interp eval sv {interp issafe k}}] {0 1 NONE}
chk s-child-exec [T {interp eval sv {catch {k eval {exec ls}} m; set m}}] {0 {invalid command name "exec"} NONE}
chk s-hide [T {interp eval sv {catch {interp hide k set} m o; list $m [dict get $o -errorcode]}}] {0 {{permission denied: safe interpreter cannot hide commands} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-expose [T {interp eval sv {catch {interp expose k exec} m o; list $m [dict get $o -errorcode]}}] {0 {{permission denied: safe interpreter cannot expose commands} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-expose2 [T {interp eval sv {catch {interp expose k exec exec} m o; list $m [dict get $o -errorcode]}}] {0 {{permission denied: safe interpreter cannot expose commands} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-invokehidden [T {interp eval sv {catch {interp invokehidden k clock seconds} m o; list $m [dict get $o -errorcode]}}] {0 {{not allowed to invoke hidden commands from safe interpreter} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-marktrusted [T {interp eval sv {catch {interp marktrusted k} m o; list $m [dict get $o -errorcode]}}] {0 {{permission denied: safe interpreter cannot mark trusted} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-reclimit-set [T {interp eval sv {catch {interp recursionlimit k 5} m o; list $m [dict get $o -errorcode]}}] {0 {{permission denied: safe interpreters cannot change recursion limit} {TCL OPERATION INTERP UNSAFE}} NONE}
chk s-reclimit-get [T {interp eval sv {interp recursionlimit k}}] {0 1000 NONE}
chk s-alias-into-self [T {interp eval sv {interp alias k a {} set ::kv 1}; interp eval sv {catch {k eval a} m o; set m}}] {0 1 NONE}
chk s-kv [T {interp eval sv {set ::kv}}] {0 1 NONE}
chk s-alias-out [T {interp eval sv {catch {interp alias k b {} nosuchcmd} m o; list $m [dict get $o -errorcode]}}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
chk s-alias-out-call [T {interp eval sv {catch {k eval b} m o; list $m [dict get $o -errorcode]}}] {0 {{invalid command name "nosuchcmd"} {TCL LOOKUP COMMAND nosuchcmd}} NONE}
chk s-alias-badmaster [T {interp eval sv {catch {interp alias k c {} nosuch} m o; list $m [dict get $o -errorcode]}}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
chk s-delete-child [T {interp eval sv {catch {interp delete k} m o; list $m [dict get $o -errorcode]}}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
chk s-exists [T {interp eval sv {list [interp exists] [interp exists k] [interp exists nope]}}] {0 {1 0 0} NONE}
chk s-slaves [T {interp eval sv {lsort [interp slaves]}}] {0 {} NONE}
U {interp eval sv {lsort [interp aliases]}}
U {interp eval sv {llength [interp hidden]}}
chk s-limit [T {interp eval sv {catch {interp limit} m o; set m}}] {0 {wrong # args: should be "interp limit path limitType ?-option value ...?"} NONE}
chk s-marktrusted-from-master [T {interp marktrusted sv}] {0 {} NONE}
chk s-after [T {list [interp issafe sv] [interp eval sv {catch {exec echo hi} m; set m}]}] {0 {0 {invalid command name "exec"}} NONE}
chk s-create-trusted-child [T {interp eval sv {interp create k2; interp issafe k2}}] {0 0 NONE}
chk s-uptail [T {interp eval sv {catch {uplevel #0 {interp hide k2 set}} m o; list $m [dict get $o -errorcode]}}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
interp delete sv
interp create s1
interp create s2
chk a-empty-master [T {interp alias s1 a1 {} list}] {0 a1 NONE}
chk a-query [T {interp alias s1 a1}] {0 list NONE}
chk a-call [T {interp eval s1 {a1 1 2}}] {0 {1 2} NONE}
chk a-again [T {interp alias s1 a1 {} concat}] {0 a1 NONE}
chk a-query2 [T {interp alias s1 a1}] {0 concat NONE}
chk a-call2 [T {interp eval s1 {a1 1 2}}] {0 {1 2} NONE}
chk a-in-path [T {interp alias s1 a2 s1 list}] {0 a2 NONE}
chk a-in-path-call [T {interp eval s1 {a2 1 2}}] {0 {1 2} NONE}
chk a-in-path-query [T {interp alias s1 a2}] {0 list NONE}
chk a-in-path-target [T {interp target s1 a2}] {0 s1 NONE}
chk a-cross-target [T {interp target s1 a1}] {0 {} NONE}
chk a-target-missing [T {interp target s1 nosuch}] {1 {alias "nosuch" in path "s1" not found} {TCL LOOKUP ALIAS nosuch}}
chk a-del-inpath [T {interp alias s1 a2 {}}] {0 {} NONE}
chk a-del-missing [T {interp alias s1 a2 {}}] {1 {alias "a2" not found} {TCL LOOKUP ALIAS a2}}
chk a-del-masterpath [T {interp alias s1 a1 s1}] {1 {wrong # args: should be "interp alias slavePath slaveCmd ?masterPath masterCmd? ?arg ...?"} {TCL WRONGARGS}}
chk a-one-extra [T {interp alias s1 a1 x}] {1 {wrong # args: should be "interp alias slavePath slaveCmd ?masterPath masterCmd? ?arg ...?"} {TCL WRONGARGS}}
chk a-two-extra [T {interp alias s1 a1 x y}] {1 {could not find interpreter "x"} {TCL LOOKUP INTERP x}}
chk a-empty-name [T {interp alias s1 {} {} list}] {0 {} NONE}
chk a-missing-path [T {interp alias nope a b}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk a-bad-master [T {interp alias s1 a9 nope list}] {1 {could not find interpreter "nope"} {TCL LOOKUP INTERP nope}}
chk a-into-parent [T {interp alias {} pa {} list; pa 7 8}] {0 {7 8} NONE}
chk a-into-parent-query [T {interp alias {} pa}] {0 list NONE}
chk a-into-parent-aliases [T {lsort [interp aliases]}] {0 pa NONE}
chk a-into-parent-target [T {interp target {} pa}] {0 {} NONE}
chk a-into-parent-del [T {interp alias {} pa {}}] {0 {} NONE}
chk a-parent-into-child [T {interp alias {} pc s1 set; pc x 3}] {0 3 NONE}
chk a-parent-into-child-r [T {interp eval s1 {set x}}] {0 3 NONE}
chk a-parent-into-child-q [T {interp alias {} pc}] {0 set NONE}
chk a-parent-into-child-t [T {interp target {} pc}] {0 s1 NONE}
chk a-parent-into-child-t2 [T {catch {interp target s1 pc} m o; set m}] {0 {alias "pc" in path "s1" not found} {TCL LOOKUP ALIAS pc}}
chk a-parent-into-child-err [T {catch {pc nosuchvar} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}] {0 {{can't read "nosuchvar": no such variable} {TCL LOOKUP VARNAME nosuchvar} {can't read "nosuchvar": no such variable
    invoked from within
"pc nosuchvar"}} {TCL LOOKUP VARNAME nosuchvar}}
U {catch {pc} m o; list $m [dict get $o -errorinfo]}
chk a-parent-into-child-ret [T {catch {pc interp alias} m o; set m}] {0 alias NONE}
chk a-parent-into-child-break [T {catch {pc break} m o; list $m [dict get $o -errorcode]}] {0 {{can't read "break": no such variable} {TCL LOOKUP VARNAME break}} {TCL LOOKUP VARNAME break}}
chk a-parent-into-child-code [T {set ::errorCode}] {0 NONE NONE}
chk a-sibling [T {interp alias s1 ab s2 set y 5}] {0 ab NONE}
chk a-sibling-call [T {interp eval s1 {ab}}] {0 5 NONE}
chk a-sibling-r [T {interp eval s2 {set y}}] {0 5 NONE}
chk a-sibling-q [T {interp alias s1 ab}] {0 {set y 5} NONE}
chk a-sibling-t [T {interp target s1 ab}] {0 s2 NONE}
chk a-sibling-t2 [T {catch {interp target s2 ab} m o; set m}] {0 {alias "ab" in path "s2" not found} {TCL LOOKUP ALIAS ab}}
chk a-sibling-t3 [T {interp eval s1 {catch {interp target {} ab} m o; set m}}] {0 {target interpreter for alias "ab" in path "" is not my descendant} NONE}
chk a-sibling-err [T {interp eval s1 {catch {ab nosuchvar} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}] {0 {{wrong # args: should be "set varName ?newValue?"} {TCL WRONGARGS} {wrong # args: should be "set varName ?newValue?"
    invoked from within
"ab nosuchvar"}} NONE}
chk a-error [T {interp alias s1 ae {} error aerr; interp eval s1 {catch {ae} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}] {0 {aerr NONE {aerr
    invoked from within
"ae"}} NONE}
chk a-error-parent-code [T {set ::errorCode}] {0 NONE NONE}
chk a-error-parent-info [T {catch {interp eval s1 ae} m o; dict get $o -errorinfo}] {0 {aerr
    invoked from within
"ae"
    invoked from within
"interp eval s1 ae"} NONE}
chk a-error-parent-ec [T {catch {interp eval s1 ae} m o; dict get $o -errorcode}] {0 NONE NONE}
chk a-error-prefix [T {interp alias s1 ae2 {} error pre {A B}; interp eval s1 {catch {ae2} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}] {0 {pre NONE {A B
    invoked from within
"ae2"}} NONE}
chk a-error-in-proc [T {interp eval s1 {proc q {} {ae}; catch {q} m o; dict get $o -errorinfo}}] {0 {aerr
    invoked from within
"ae"
    (procedure "q" line 1)
    invoked from within
"q"} NONE}
U {interp eval s1 {proc q2 {} {ae}; catch {q2} m o; dict get $o -errorstack}}
chk a-unknown-target [T {interp alias s1 au {} nosuchcmd; interp eval s1 {catch {au x} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}] {0 {{invalid command name "nosuchcmd"} {TCL LOOKUP COMMAND nosuchcmd} {invalid command name "nosuchcmd"
    invoked from within
"au x"}} {TCL LOOKUP COMMAND nosuchcmd}}
U {interp alias s1 aq {} llength; interp eval s1 {catch {aq} m o; list $m [dict get $o -errorinfo]}}
chk a-nsname [T {interp alias s1 {::ns::al} {} list}] {0 ::ns::al NONE}
chk a-nsname-call [T {interp eval s1 {catch {ns::al 1} m o; set m}}] {0 1 NONE}
chk a-nsname-call2 [T {interp eval s1 {catch {{::ns::al} 1} m o; set m}}] {0 1 NONE}
chk a-nsname-q [T {interp alias s1 {::ns::al}}] {0 list NONE}
chk a-ns-target [T {interp alias s1 nt {} set ::nsv; interp eval s1 {nt 4}}] {0 4 NONE}
chk a-ns-target-r [T {set ::nsv}] {0 4 NONE}
chk a-ns-var-err [T {interp eval s1 {catch {nt} m o; list $m [dict get $o -errorcode]}}] {1 {key "-errorcode" not known in dictionary} {TCL LOOKUP DICT -errorcode}}
chk a-prefix [T {interp alias s1 pf {} list A {b c}; interp eval s1 {pf "d e" {f g}}}] {0 {A {b c} {d e} {f g}} NONE}
chk a-prefix-literal [T {interp alias s1 pl {} set p "[expr 1 + 1] \$x"; interp eval s1 {pl}}] {0 {2 $x} NONE}
chk a-prefix-literal-r [T {interp eval s1 {set p}}] {1 {can't read "p": no such variable} {TCL LOOKUP VARNAME p}}
chk a-replace-proc [T {interp eval s1 {proc rp {x} {return "rp:$x"}}; interp alias s1 rp {} list}] {0 rp NONE}
chk a-replace-proc-call [T {interp eval s1 {rp 3}}] {0 3 NONE}
chk a-replace-alias [T {interp alias s1 rp s1 concat}] {0 rp NONE}
chk a-replace-alias-call [T {interp eval s1 {rp 3 4}}] {0 {3 4} NONE}
chk a-recreate [T {interp alias s1 rc {} list; interp alias s1 rc {} concat}] {0 rc NONE}
chk a-recreate-call [T {interp eval s1 {rc 3 4}}] {0 {3 4} NONE}
chk a-list [T {lsort [interp aliases s1]}] {0 {{} ::ns::al a1 ab ae ae2 aq au nt pf pl rc rp} NONE}
U {interp hide s1 set; interp eval s1 {catch {a1 1} m o; set m}}
U {interp alias s1 ah s1 set hh 1; interp eval s1 {catch {ah} m o; set m}}
chk a-invokehidden-alias [T {interp invokehidden s1 a1 5}] {1 {invalid hidden command name "a1"} {TCL LOOKUP HIDDENTOKEN a1}}
chk a-expose-set [T {interp expose s1 set}] {0 {} NONE}
chk a-expose-set2 [T {interp eval s1 {catch {set hh} m o; set m}}] {0 {can't read "hh": no such variable} NONE}
chk a-hide-alias [T {interp hide s1 a1}] {0 {} NONE}
chk a-hidden-list [T {lsort [interp hidden s1]}] {0 a1 NONE}
chk a-aliases-after-hide [T {lsort [interp aliases s1]}] {0 {{} ::ns::al a1 ab ae ae2 ah aq au nt pf pl rc rp} NONE}
chk a-hidden-alias-call [T {interp eval s1 {catch {a1 1} m o; set m}}] {0 {invalid command name "a1"} NONE}
chk a-invoke-hidden-alias [T {interp invokehidden s1 a1 6}] {0 6 NONE}
chk a-expose-alias [T {interp expose s1 a1}] {0 {} NONE}
chk a-exposed-alias-call [T {interp eval s1 {a1 7}}] {0 7 NONE}
chk a-rename-alias-in-child [T {interp eval s1 {rename a1 a1r}}] {0 {} NONE}
chk a-renamed-call [T {interp eval s1 {a1r 8}}] {0 8 NONE}
chk a-renamed-aliases [T {lsort [interp aliases s1]}] {0 {{} ::ns::al a1 ab ae ae2 ah aq au nt pf pl rc rp} NONE}
chk a-renamed-path [T {rename s1 s1r}] {0 {} NONE}
chk a-renamed-path-eval [T {s1r eval {a1r 10}}] {0 10 NONE}
chk a-renamed-path-exists [T {list [interp exists s1] [interp exists s1r]}] {0 {1 0} NONE}
chk a-renamed-path-interp [T {catch {interp eval s1 {a1r 11}} m o; set m}] {0 11 NONE}
chk a-renamed-path-alias [T {catch {interp alias s1 newa {} list} m o; set m}] {0 newa NONE}
chk a-renamed-path-alias-call [T {s1r eval {newa 12}}] {0 12 NONE}
chk a-renamed-path-hide [T {s1r hide a1r; lsort [s1r hidden]}] {0 a1r NONE}
chk a-renamed-path-expose [T {s1r expose a1r}] {0 {} NONE}
U {lsort [interp slaves]}
chk a-renamed-back [T {rename s1r s1}] {0 {} NONE}
chk a-renamed-back-eval [T {s1 eval {newa 13}}] {0 13 NONE}
chk a-deep-create [T {interp eval s1 {interp create g}}] {0 g NONE}
chk a-deep-alias-top [T {interp alias {s1 g} ga {} set ::gg 8}] {0 ga NONE}
chk a-deep-alias-call [T {interp eval {s1 g} {ga}}] {0 8 NONE}
chk a-deep-alias-r [T {set ::gg}] {0 8 NONE}
chk a-deep-alias-call2 [T {interp eval s1 {catch {ga} m o; set m}}] {0 {invalid command name "ga"} NONE}
chk a-deep-alias-q [T {interp alias {s1 g} ga}] {0 {set ::gg 8} NONE}
chk a-deep-alias-q2 [T {interp eval s1 {interp alias g ga}}] {0 {set ::gg 8} NONE}
chk a-deep-alias-t [T {interp target {s1 g} ga}] {0 {} NONE}
chk a-deep-alias-t2 [T {interp eval s1 {catch {interp target g ga} m o; set m}}] {0 {target interpreter for alias "ga" in path "g" is not my descendant} NONE}
chk a-deep-alias-t3 [T {interp eval {s1 g} {catch {interp target {} ga} m o; set m}}] {0 {target interpreter for alias "ga" in path "" is not my descendant} NONE}
chk a-deep-into-s1 [T {interp alias {s1 g} gb s1 set ggb 3}] {0 gb NONE}
chk a-deep-into-s1-call [T {interp eval {s1 g} {gb}}] {0 3 NONE}
chk a-deep-into-s1-r [T {interp eval s1 {set ggb}}] {0 3 NONE}
chk a-deep-into-s1-q [T {interp alias {s1 g} gb}] {0 {set ggb 3} NONE}
chk a-deep-into-s1-t [T {interp eval s1 {catch {interp target g gb} m o; set m}}] {0 {} NONE}
chk a-deep-into-self [T {interp alias {s1 g} gc {s1 g} set ggc 4}] {0 gc NONE}
chk a-deep-into-self-call [T {interp eval {s1 g} {gc}}] {0 4 NONE}
chk a-deep-into-self-r [T {interp eval {s1 g} {set ggc}}] {0 4 NONE}
chk a-deep-into-sibling [T {interp create s3; interp alias {s1 g} gd s3 set q 1}] {0 gd NONE}
chk a-deep-into-sibling-call [T {interp eval {s1 g} {gd}; interp eval s3 {set q}}] {0 1 NONE}
chk a-deep-into-sibling-t [T {interp eval s1 {catch {interp target g gd} m o; set m}}] {0 {target interpreter for alias "gd" in path "g" is not my descendant} NONE}
chk a-deep-from-sibling [T {interp eval s1 {catch {interp alias g ge s2 set q 2} m o; set m}}] {0 {could not find interpreter "s2"} NONE}
chk a-deep-make-from-inside [T {interp eval s1 {interp alias g gf s2 set q 3}}] {1 {could not find interpreter "s2"} {TCL LOOKUP INTERP s2}}
chk a-deep-make-from-inside-call [T {catch {interp eval {s1 g} {gf}} m o; set m}] {0 {invalid command name "gf"} {TCL LOOKUP COMMAND gf}}
chk a-deep-aliases [T {lsort [interp aliases {s1 g}]}] {0 {ga gb gc gd} NONE}
chk a-deep-delete-sibling [T {interp delete s2; lsort [interp aliases {s1 g}]}] {0 {ga gb gc gd} NONE}
chk a-deep-after-delete [T {interp eval {s1 g} {catch {gb} m o; set m}}] {0 3 NONE}
chk a-deep-after-delete-aliases [T {lsort [interp aliases s1]}] {0 {{} ::ns::al a1 ae ae2 ah aq au newa nt pf pl rc rp} NONE}
interp eval s1 {proc P {script} {
    set c [catch {uplevel 1 $script} r o]
    set e [catch {dict get $o -errorcode} e]
    set i [catch {dict get $o -errorinfo} i]
    list $c $r [expr {$e ? "" : $e}] [expr {$i ? "" : [string map {"\n" "^"} $i]}]
}}
chk a-deep-err [T {interp eval s1 {P {interp eval g {proc p {} {gb 1}; catch {p} m o; dict get $o -errorinfo}}}}] {0 {0 {wrong # args: should be "set varName ?newValue?"
    invoked from within
"gb 1"
    (procedure "p" line 1)
    invoked from within
"p"} {} {}} NONE}
chk a-deep-err2 [T {interp eval s1 {P {interp alias g gx {} error deep; interp eval g {catch {gx} m o; list $m [dict get $o -errorinfo]}}}}] {0 {0 {deep {deep
    invoked from within
"gx"}} {} {}} NONE}
chk a-deep-err3 [T {interp eval s1 {P {interp alias g gx {} error deep; interp eval g {catch {gx} m o}; dict get $o -errorinfo}}}] {0 {0 {could not find interpreter "s2"
    while executing
"interp alias g ge s2 set q 2"} {} {}} NONE}
chk l-self [T {interp alias {} z {} z}] {1 {cannot define or rename alias "z": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-self-ec [T {set ::errorCode}] {0 NONE NONE}
chk l-self-call [T {catch {z} m o; set m}] {0 {invalid command name "z"} {TCL LOOKUP COMMAND z}}
chk l-2 [T {interp create l2; interp alias l2 x {} x}] {0 x NONE}
chk l-2b [T {interp alias l2 x {} x2}] {0 x NONE}
chk l-3 [T {interp create l3; interp alias l3 x l3 x}] {1 {cannot define or rename alias "x": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-4 [T {interp create l4; interp alias l4 x {} y}] {0 x NONE}
chk l-4b [T {interp alias {} y l4 x}] {1 {cannot define or rename alias "y": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-4c [T {interp alias {} y l4 z}] {0 y NONE}
chk l-5 [T {interp create l5; interp alias l5 x l5 y}] {0 x NONE}
chk l-5b [T {interp alias l5 y l5 x}] {1 {cannot define or rename alias "y": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-6 [T {interp alias {} l5z l5 q}] {0 l5z NONE}
chk l-6b [T {interp alias l5 q {} l5z}] {1 {cannot define or rename alias "q": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-7 [T {interp create m1; interp create m2; interp alias m1 a m2 b}] {0 a NONE}
chk l-7b [T {interp alias m2 b m1 a}] {1 {cannot define or rename alias "b": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-8 [T {interp create l8; interp alias l8 x {} l8y}] {0 x NONE}
chk l-8b [T {interp alias {} l8y l8 x}] {1 {cannot define or rename alias "l8y": would create a loop} {TCL OPERATION INTERP ALIASLOOP}}
chk l-9 [T {interp create l9; interp alias l9 x {} r2; rename r2 x}] {1 {can't rename "r2": command doesn't exist} {TCL LOOKUP COMMAND r2}}
chk l-9-ec [T {set ::errorCode}] {0 NONE NONE}
chk l-9-call [T {catch {interp eval l9 x} m o; set m}] {0 {invalid command name "r2"} {TCL LOOKUP COMMAND r2}}
chk l-10 [T {interp alias {} r3 l9 x}] {0 r3 NONE}
chk l-10b [T {rename r3 l9x}] {0 {} NONE}
chk l-10c [T {rename l9x r3}] {0 {} NONE}
interp create e1
interp create e2
chk x-1 [T {interp alias e1 t d2 set q 1}] {0 t NONE}
chk x-2 [T {interp alias e2 u d1 set q 2}] {1 {could not find interpreter "d1"} {TCL LOOKUP INTERP d1}}
chk x-3 [T {interp alias e1 v {} set q 3}] {0 v NONE}
chk x-4 [T {interp delete e2; lsort [interp aliases e1]}] {0 {t v} NONE}
chk x-5 [T {interp eval e1 {catch {t} m o; set m}}] {0 1 NONE}
chk x-6 [T {catch {interp alias e1 v {} d2 set} m o; set m}] {0 v NONE}
U {interp delete e1; interp aliases}
chk x-8 [T {interp alias {} w {} set q 4; interp delete {}}] {1 {cannot delete the current interpreter} {TCL OPERATION INTERP DELETESELF}}
U {interp create e3; interp alias {} w2 d3 set q 5; interp delete e3; interp aliases}
chk x-10 [T {catch {w2} m o; set m}] {0 5 NONE}
U {interp create e4; interp alias e4 wa {} interp delete e4; interp eval e4 wa}
U {interp exists e4}
U {interp delete e4}
chk x-14 [T {catch {w} m o; set m}] {0 4 NONE}

# 335 lines, the chk ones are the checks.
