foreach cmd {
 {binary format c* {0x50 0x52}}
 {set x [binary format f* {1 -1 2 -2 0}]; binary scan $x f* bla; set bla}
 {binary scan x a[format %lld 0x10000000000000000] r}
 {binary scan [binary format H* 7f7fffff] R fltmax; binary scan [binary format H* 47effffff0000000] Q round_to_fltmax; binary scan [binary format R $round_to_fltmax] R fltmax1; expr {$fltmax eq $fltmax1}}
} {
 if {[catch $cmd r]} { puts "ERR $r" } else { puts "OK <$r>" }
}
