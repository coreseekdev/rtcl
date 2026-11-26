# C16（9.1 对齐批 B1，2026-11-26）：expr 错误的帧关键词 + 9.1 操作数
# 分类消息/errorCode。oracle（tclsh 9.1.0）钉形（顶层逐例 catch，避开
# proc 行号记账的既有分歧面，只钉本批语义）：
# - 折叠常量子树的错误（编译期折叠失败、延迟到运行期抛出）→ 首帧
#   关键词 `invoked from within`；含运行期值（变量/命令替换/函数调用）
#   的节点错误 → `while executing`。布尔语境（&&/||/?:/if/while）的
#   转换错误只在**整链全常量**时才是折叠错误（`{"a" && 1}` inner，
#   `{"a" && $z}` while executing）。
# - 9.1 消息：`cannot use non-numeric string "a" as left operand of "+"`
#   （引值 + 左/右；一元无侧）、`cannot use floating-point value "4.0" …`、
#   `cannot use non-numeric floating-point value "NaN" …`、
#   `cannot use a list as …`（多元素 list 值）。
# - errorCode：ARITH DOMAIN {non-numeric string}/{floating-point value}/
#   {non-numeric floating-point value}/list/{exponentiation of zero by
#   negative power}；expected boolean → TCL VALUE NUMBER；NaN →
#   TCL VALUE DOUBLE NAN；expr 数学函数 expected number → TCL VALUE NUMBER。
# - ~4.0 一元按位反在 9.1 是错误（8.6 时代截断为 int 再取反 = -5）。
# - 条件语境（if/while/for）不做结果规范化：`while {NaN}` 报转换错
#   （floating point value is Not a Number），`expr {NaN}` 才是
#   canonicalization 的 domain error（bn7 钉形保持）。
set z 0
catch {expr {1/0}} m e; puts "A m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1%0}} m e; puts "B m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {0**-1}} m e; puts "C m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1/$z}} m e; puts "D m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {sqrt(-1)}} m e; puts "E m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1/(2-2)}} m e; puts "K m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
set zz 1
catch {expr {$zz + 1/0}} m e; puts "L m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {$zz * (1/0)}} m e; puts "M m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {"a" + 1}} m e; puts "N m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {(1/0) + $zz}} m e; puts "O m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1/0 + $zz}} m e; puts "S m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {wide(1)/0}} m e; puts "V m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {(1/0)}} m e; puts "W m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1/0 == 1}} m e; puts "Z m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {[expr {1/0}] + $zz}} m e; puts "AB m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1/0 + sqrt(4)}} m e; puts "AC m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {int("x")}} m e; puts "AD m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {0 && (1/0)}} m; puts "SC0=[expr {0 && (1/0)}] SC1=[expr {1 || (1/0)}] TERN=[expr {0 ? (1/0) : 5}]"
catch {expr {1%0.0}} m e; puts "MODF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1.5&1}} m e; puts "ANDF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {true+1}} m e; puts "BW m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {3**268435456}} m e; puts "EXP m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1 << -1}} m e; puts "SH m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {if {1/0} {}} m e; puts "IFF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {if {1/$z} {}} m e; puts "IFR m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {while {1/0} {}} m e; puts "WF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {set xx [expr {1/0}]} m e; puts "SB m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {"" + 1}} m e; puts "EMP m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1 + NaN}} m e; puts "NAN m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {Inf << 1}} m e; puts "INF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {~4.0}} m e; puts "TLDF m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
set a a
catch {expr {-$a}} m e; puts "UNA m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {!NaN}} m e; puts "NOTN m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
set l {1 2}
catch {expr {!$l}} m e; puts "NOTL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {~$l}} m e; puts "TLDL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {-$l}} m e; puts "NEGL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {$l + 1}} m e; puts "LADD m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {"a" && 1}} m e; puts "BA m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {1 && "a"}} m e; puts "BAL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
set z1 1
catch {expr {$z1 && "a"}} m e; puts "BRT m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {"a" && $z1}} m e; puts "BRT2 m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {$z1 && (1/0)}} m e; puts "BFLD m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {"a" ? 1 : 2}} m e; puts "BTE m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {$l ? 1 : 2}} m e; puts "BTL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {if {NaN} {}} m e; puts "IFN m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {if {$l} {}} m e; puts "IFL m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {if {"a"} {}} m e; puts "IFB m=$m ec=[dict get $e -errorcode]"; puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
catch {expr {NaN}} m; puts "NANC m=$m"
catch {while {NaN} {break}} mm; puts "WHN=$mm"
puts "and0=[expr {0 && (1/0)}] or1=[expr {1 || (1/0)}] tern=[expr {0 ? (1/0) : 5}]"
