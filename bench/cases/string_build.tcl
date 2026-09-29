# string building: append + string ops
set s ""
for {set i 0} {$i < 2000} {incr i} {
    append s "item-$i;"
}
puts [string length $s]
puts [string toupper [string range $s 0 99]]
