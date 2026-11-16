proc p {} {
    set a 0
    incr a [nosuchcmd]
}
if [catch p m] { puts "E:$m" }
puts $errorInfo
