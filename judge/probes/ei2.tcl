namespace eval ee {
    puts "B:[info level 0]"
}
proc pp {x y} { info level 0 }
puts "C:[pp a b]"
puts "D:[::pp q r]"
proc outer {} { inner 1 2 }
proc inner {a b} { info level 0 }
puts "E:[outer]"
proc cc {} { catch {info level 0} m; return $m }
puts "G:[cc]"
namespace eval mm info level 0
namespace eval uu { uplevel 0 {puts "H:[info level 0]"} }
puts "F2:[info level 0]"
