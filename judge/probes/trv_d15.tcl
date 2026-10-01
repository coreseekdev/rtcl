variable x notrace
proc callback {old - -} {
    variable x "wrote:$old"
    puts "cb-ran"
}
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete [namespace code callback]
puts "script=[namespace code callback]"
namespace delete ::foo
puts "x=[set x]"
puts "gx=[set ::x]"
