# 1: info level 0 at global file scope
puts "A:[info level 0]"
# 2: inside ns eval braced script
namespace eval ee {
    puts "B:[info level 0]"
}
# 3: proc, normal call
proc pp {x y} { info level 0 }
puts "C:[pp a b]"
# 4: proc, ::-qualified call
puts "D:[::pp q r]"
# 5: nested proc frames
proc outer {} { inner 1 2 }
proc inner {a b} { info level 0 }
puts "E:[outer]"
puts "F:[info level]"
# 6: info level 0 inside catch inside proc
proc cc {} { catch {info level 0} m; return $m }
puts "G:[cc]"
# 7: ns eval multi-arg form
namespace eval mm info level 0
# 8: uplevel context
proc up {} { info level 0 }
namespace eval uu { uplevel 0 {puts "H:[up]" } }
