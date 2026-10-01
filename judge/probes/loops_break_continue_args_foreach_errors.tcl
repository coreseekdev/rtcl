# agent-e-loops: decoded tclsh 8.6.17 details for break/continue arity and
# foreach list-parse / loop-variable-write failures.
# Oracle output (label|catchCode|result):
#   break foo / continue foo / break 2 -> "wrong # args: should be
#     \"break\"/\"continue\"", errorCode TCL WRONGARGS.
#   foreach malformed varlist or list  -> the list scanner's error
#     (`list element in braces followed by "x" instead of space`,
#     errorCode TCL VALUE LIST JUNK), framed with the foreach command.
#   foreach writing an array element name -> `can't set "a": variable is
#     array` with a dedicated `(setting foreach loop variable "a")` frame
#     (foreach-1.14), errorCode TCL WRITE VARNAME.
# NB: errorInfo lines marked stale-... carry ::errorInfo left over from an
# earlier probe in the same interp (errorCode/errorInfo persist past catch).
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t brk-args {catch {break foo} m; set m}
t cont-args {catch {continue foo} m; set m}
t brk-level {catch {break 2} m; set m}
t brk-level0 {catch {break 0} m; set m}
t brk-info {catch {break foo} m; set ::errorInfo}
t brk-ec {catch {break foo} m; set ::errorCode}
t cont-ec {catch {continue foo} m; set ::errorCode}
t fe-varlist-junk {catch {foreach {{a}{b}} {1 2 3} {}} m; list $m [set ::errorCode]}
t fe-datalist-junk {catch {foreach a {{1 2}3} {}} m; list $m [set ::errorCode]}
t fe-varlist-junk-info {catch {foreach {{a}{b}} {1 2 3} {}} m; set ::errorInfo}
t fe-setfail {catch {unset a}; set a(0) 44; catch {foreach a {1 2 3} {}} m; list $m [set ::errorCode]}
t fe-setfail-info {catch {unset a}; set a(0) 44; catch {foreach a {1 2 3} {}} m; set ::errorInfo}
t fe-setfail-2var {catch {unset b}; set b(0) 9; catch {foreach {b c} {1 2 3} {}} m; set ::errorInfo}
t fe-emptyvars {catch {foreach {} {} {}} m; list $m [set ::errorCode]}
t lm-ret0 {lmap i {a b {{c d} e} {123 {{x}}}} { return -level 0 $i }}
