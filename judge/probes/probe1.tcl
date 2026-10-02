# 1. info commands 命名空间语义
namespace eval foo { proc p {} {} }
namespace eval foo { puts "IN_FOO: [llength [info commands]] has_p=[expr {[lsearch -exact [info commands] p]>=0}]" }
puts "TOP: [llength [info commands]]"
puts "TCLNS: [llength [info commands ::tcl::*]]"
puts "FOO_PAT: [info commands foo::*]"
puts "RELPAT: [llength [info commands p]]"
# 4. wrong-args errorCode: proc vs builtin
proc pp {a b} {}
catch {pp 1}
puts "PROC_WA: <$errorCode>"
catch {set}
puts "BUILTIN_WA: <$errorCode>"
# 5. errorCode 存在性
puts "EC_EXISTS_FRESH: [info exists errorCode] val=<${errorCode}>"
catch {error boom}
puts "EC_AFTER_ERR: <$errorCode>"
# 6. errorInfo 最外层帧
proc q {} {error boom2}
catch {q}
puts "EI_CATCH: $errorInfo"
# 8a. 非法 POSIX 字符类
catch {regexp {[:foo:]} x} m
puts "CC_ERR: <$m> <$errorCode>"
# 8b. hex float
catch {expr 0x1.8p3} r
puts "HEXF: <$r>"
