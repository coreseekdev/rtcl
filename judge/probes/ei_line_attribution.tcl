# errorLine 归属矩阵（tclsh errorLine 只在日志事件处更新；构造出口帧
# 读最内层已记日志命令的行；括号内容在编译 proc 体里内联 = 体绝对行。
# 2026-11-14 errorInfo 帧组轮；procedure 帧名 = 调用词同文件钉形）。
proc p {} {
    set x [expr {1 +
        [nosuch b]}]
}
catch {p} m
puts "A:$::errorInfo"
set ::errorInfo {}
proc q {} {
    set x [
        nosuch]
}
catch {q} m
puts "B:$::errorInfo"
set ::errorInfo {}
catch {uplevel 0 {set x [
    nosuch]}} m
puts "C:$::errorInfo"
set ::errorInfo {}
catch {namespace eval ne {set x [
    nosuch]}} m
puts "D:$::errorInfo"
set ::errorInfo {}
catch {eval {set x [
    nosuch]}} m
puts "E:$::errorInfo"
set ::errorInfo {}
proc p4 {} {

    error boom
}
catch {p4} m
puts "F:$::errorInfo"
set ::errorInfo {}
namespace eval n1 { proc show {} { nosuch } }
catch {n1::show} m
puts "G:$::errorInfo"
set ::errorInfo {}
catch {::n1::show} m
puts "H:$::errorInfo"
set ::errorInfo {}
namespace eval n1 { catch {show} }
puts "I:$::errorInfo"
set ::errorInfo {}
proc plain {} { nosuch }
catch {plain} m
puts "J:$::errorInfo"
