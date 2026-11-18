# 浮点取模非法 → 宿主全语义报错（lane on 也同文）
if {[catch {expr {1.5 % 2}} msg]} {
    puts "ERR: $msg"
} else {
    puts "NO-ERR: $msg"
}
if {[catch {expr {1.0 & 2.0}} msg]} {
    puts "ERR: $msg"
} else {
    puts "NO-ERR: $msg"
}
