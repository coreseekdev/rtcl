proc gp {} {}
namespace eval e1 {proc c1 {} {}}
namespace eval p1 {}
proc p1::sub {} {}
puts [info commands gp]
puts [info commands e1::*]
puts [info commands p1*]
puts [info commands ::p1*]
puts [info commands]
