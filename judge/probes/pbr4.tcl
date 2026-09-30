set a [binary format R 3.4028235677973366e+38]
binary scan $a H* h1
puts <$h1>
binary scan $a R v
puts <$v>
set b [binary format R $v]
binary scan $b H* h2
puts <$h2>
puts <[expr {$v eq 3.4028234663852886e+38}]>
