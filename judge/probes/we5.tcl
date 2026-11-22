proc boomq {} {error "boomq-msg"}
proc p {} {
    set q [boomq]
}
catch {p}
puts "L1<<$::errorInfo>>"
namespace eval nn {
    set r [boomq]
}
puts "L2-done"
