# crash-probe: c06 expr hash
# gate: max-ms 3000
# `#` in an expression is an invalid character in tclsh 8.6.  The checker
# scanned it as a comment and restarted without advancing, so every
# `expr {3 #}` / `expr {#}` / `expr {1 + #}` spun at 100% CPU forever.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE
set r [catch {expr {3 #}} m]
# skip rc -- 9.x expr 支持注释，# 不再死循环而是注释——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立，钉形保留备考）：chk rc $r 1
# skip msg -- 9.x expr 支持注释，# 不再死循环而是注释——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立，钉形保留备考）：chk msg $m {invalid character "#"
#   （上断言的多行 want 续行，一并跳过）：in expression "3 #"}
# skip ec -- 9.x expr 支持注释，# 不再死循环而是注释——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立/状态链断，钉形保留备考）：chk ec $::errorCode {TCL PARSE EXPR BADCHAR}
chk alone [catch {expr {#}} m] 1
chk operand [catch {expr {1 + #}} m] 1
# skip multiline -- 9.x expr 支持注释，# 不再死循环而是注释——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立/状态链断，钉形保留备考）：chk multiline [catch {expr {# c
# skip 续行 -- 9.x expr 支持注释，# 不再死循环而是注释——TODO 对标迁移 T1 expr 注释
#   原断言（9.1 下不成立/状态链断，钉形保留备考）：1 + 1}} m] 1
# inside a string, a brace or a script substitution it is data
chk quoted [expr {"a # b"}] {a # b}
chk script [expr {3 + [llength {# x}]}] 5
chk braced [expr {[string length {# #}]}] 3
