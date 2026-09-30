namespace eval t2 {
    variable v in-t2
    proc show {script} { return [uplevel 1 $script] }
    proc show0 {script} { return [uplevel #0 $script] }
    proc show2 {script} { return [uplevel 2 $script] }
}
namespace eval t3 {
    proc outer {script} { return [t2::show $script] }
    proc outer2 {script} { return [t2::show2 $script] }
}
puts "A:[t2::show {namespace current}]"
puts "B:[t2::show {info exists t2::v}]"
puts "C:[t2::show {set v}]"
puts "D:[t2::show0 {namespace current}]"
puts "E:[t3::outer {namespace current}]"
puts "F:[t3::outer2 {namespace current}]"
puts "G:[t2::show {info level}]"
puts "H:[t2::show0 {info level}]"
namespace eval t2 {
    set w local-w
    proc peek {} { return [uplevel 1 {set w}] }
}
puts "I:[t2::peek]"
catch {t2::show {set novar}} m
puts "J:$m"
