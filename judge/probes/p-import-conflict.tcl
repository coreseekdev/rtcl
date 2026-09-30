namespace eval test_ns_export {
    namespace export cmd1 cmd2 cmd3
    proc cmd1 {args} {return "cmd1: $args"}
    proc cmd2 {args} {return "cmd2: $args"}
    proc cmd3 {args} {return "cmd3: $args"}
    proc cmd4 {args} {return "cmd4: $args"}
}
namespace eval test_ns_import {
    namespace export cmd1 cmd2
    namespace import ::test_ns_export::*
}
namespace import test_ns_import::cmd*
puts "A<<[lsort [info commands cmd*]]>>"
rename cmd1 ""
puts "B<<[info commands cmd?]>>"
namespace forget test_ns_import::cmd?
puts "C<<[info commands cmd?]>> [lsort [info commands test_ns_import::*]]>>"
proc cmd1 {x y} {return [expr {$x+$y}]}
puts "D<<[catch {namespace import test_ns_import::cmd?} msg] $msg [cmd1 3 5]>>"
namespace import -force test_ns_import::cmd?
puts "E<<[cmd1 3 5] [test_ns_import::cmd1 3 5] [namespace origin cmd1]>>"
