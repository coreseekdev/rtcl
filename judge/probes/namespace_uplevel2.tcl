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
namespace eval t2 {
    puts "A:[show {namespace current}]"
    puts "B:[show {set v}]"
    catch {show {set novar}} m
    puts "C:$m"
    puts "D:[show0 {namespace current}]"
    puts "E:[show {info level}]"
    puts "F:[show0 {info level}]"
}
namespace eval t3 {
    puts "G:[outer {namespace current}]"
    puts "H:[outer2 {namespace current}]"
    puts "I:[outer {info level}]"
    set w outer-w
    proc peekw {} { return [t2::show {set w}] }
}
catch {t3::peekw} m2
puts "J:$m2"
namespace eval t3 {
    variable w3 v3
    proc peekw3 {} { return [t2::show {set w3}] }
}
catch {t3::peekw3} m3
puts "K:$m3"
