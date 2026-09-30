puts <[format %lld 0x10000000000000000]>
puts <[binary scan [binary format H* 7f7fffff] R fltmax; set fltmax]>
puts <[binary scan [binary format H* 47effffff0000000] Q r2; set r2]>
puts <[binary format R 3.4028234663852886e+38]>
