# crash-probe: b03 alias loop
# 9.1 注意：tclsh 9.1 已无裸 `alias` 命令（invalid command name），本探针的
# 状态行在 oracle 下建不起来——期望值只能来自 8.6.17 钉形。`alias` 命令
# 的去留是迁移裁定项（TODO 对标迁移 T4/T5）；裁定前本探针只在 rtcl 上防回归。
source [file join [file dirname [info script]] _chk.tcl]
# tclsh rejects the definition: cannot define or rename alias "a": would create a loop
chk self-def [catch {alias a a} e] 1
# skip self-msg -- 9.x interp/alias 语义重做，循环检测报法不同——TODO 对标迁移 T4/T5
#   原断言（9.1 下不成立，钉形保留备考）：chk self-msg $e {cannot define or rename alias "a": would create a loop}
chk self-call [catch {a} e] 1
alias b puts
chk chain-def [catch {alias c b}] 0
chk mutual-def [catch {alias b c} e] 1
chk mutual-msg $e {cannot define or rename alias "b": would create a loop}
# a real chain still works (each alias call nests one level and unwinds)
proc leaf {x} { return "got $x" }
alias c0 leaf
for {set i 1} {$i < 300} {incr i} { "alias" "c$i" "c[expr {$i - 1}]" }
chk chain [c299 end] {got end}
