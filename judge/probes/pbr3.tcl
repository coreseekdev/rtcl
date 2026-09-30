set x 0x10000000000000000
puts <$x>
puts <[expr {$x}][expr {$x+0}]>
puts <[format %lld $x]>
puts <[format %d 18446744073709551616]>
puts <[catch {format %lld xyz} e]; set e>
