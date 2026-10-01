namespace eval ::ref {}
set ::ref::var1 AAA
trace add variable ::ref::var1 unset doTrace3
set ::ref::var2 BBB
proc doTrace3 {vtraced vidx op} {
    puts "cb: all=[info vars] ref=[info vars ::ref::*] v2=[catch {set ::ref::var2} m2]::$m2"
}
namespace delete ::ref
rename doTrace3 {}
puts "after: [info vars ::ref::*]"
