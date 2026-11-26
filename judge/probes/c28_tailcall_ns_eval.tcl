# C28（9.1 对齐批 B1，2026-11-26）：namespace eval 体里的 tailcall 非法。
# oracle（tclsh 9.1.0）实测：
# - `proc p {} { namespace eval ns { tailcall z } }` →
#   `tailcall can only be called from a proc, lambda or method`
#   errorCode TCL TAILCALL ILLEGAL（原 rtcl 照常执行 tailcall，z 在 ns
#   退出后查找失败报 invalid command name）；errorInfo 带
#   `(in namespace eval "::ns" script line N)` 中间帧。
# - 顶层裸 tailcall 同消息同码（rtcl 原 errorCode NONE）。
# - proc 体/apply 体里的 tailcall 仍合法（目标未定义时才报
#   invalid command name）。
# - ns-eval 体结束后（帧回到 proc 语境）tailcall 仍合法。
catch {tailcall x} m e
puts "top: m=$m ec=[dict get $e -errorcode]"
puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
proc p {} { namespace eval ns { tailcall z } }
catch {p} m e
puts "ns: m=$m ec=[dict get $e -errorcode]"
puts "  ei=[regsub -all {\n +} $::errorInfo { | }]"
proc q {} { tailcall w }
catch {q} m
puts "proc-legal: c=$m"
catch {apply {{} { tailcall v }} } m
puts "apply-legal: c=$m"
proc r2 {} {
    namespace eval n2 {
        variable v 5
        proc helper {} { return h }
    }
    tailcall n2::helper
}
catch {r2} m
puts "tailcall-after-ns-body: c=$m"
