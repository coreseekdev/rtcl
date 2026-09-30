set x GLOBAL
namespace eval t1 {
    variable x
    puts "bare-read:[catch {set x} m] $m"
    puts "bare-exists:[info exists x]"
    puts "bare-which:[namespace which -variable x]"
    set x 9
    puts "bare-then-set:[set x] [namespace which -variable x]"
}
puts "global-after-t1:[set x]"
namespace eval t2 {
    variable y 5
    unset y
    puts "t2-read:[catch {set y} m] $m"
    puts "t2-exists:[info exists y]"
    puts "t2-which:[namespace which -variable y]"
}
