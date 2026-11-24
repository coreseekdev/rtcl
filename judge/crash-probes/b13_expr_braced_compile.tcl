# crash-probe: b13 expr braced compile
source [file join [file dirname [info script]] _chk.tcl]
# A braced expr with 100000 parens: the compile-time descent used to run
# the native stack out at the `proc`. The definition now succeeds (the
# expression is left to the run-time evaluator, like any other expr
# error), and the call reports the nesting limit.
set n 100000
set body "expr {"
append body [string repeat "(" $n] 1 [string repeat ")" $n] "}"
chk define [catch {proc p {} $body} e] 0
chk call [catch {p} e] 1
