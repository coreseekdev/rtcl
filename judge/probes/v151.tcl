proc A { name } {
    upvar $name var
    set var $name
}
namespace eval test A useSomeUnlikelyNameHere
namespace eval test unset useSomeUnlikelyNameHere
puts "R15-done"
