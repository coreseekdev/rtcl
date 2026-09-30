foreach cmd {
 {binary format a foo}
 {binary format c2 {0x50 0x52 0x53}}
 {binary format c2 {65}}
 {binary format c1 {65 66}}
 {binary format ax*a}
 {binary format {a*  @0  a2 @* a*} foobar ab blat}
 {binary format a*X*a foo z}
 {binary scan abc i}
 {binary scan abcdefg a3X*a3 arg1 arg2}
 {binary scan x a18446744073709551615 r}
 {binary decode hex "61 61"}
 {binary decode hex "6"}
 {binary decode base64 -}
 {binary encode base64 -}
 {binary encode uuencode -maxlen 30 1234567890123456}
 {binary decode uuencode "!86"}
 {binary scan 123456 c2 v; set v}
 {binary scan abc aa v1 v2; list $v1 $v2}
 {binary format f1 {1.5 2.5}}
 {binary scan abcdef a5c v w; info exists v}
} {
 if {[catch $cmd r]} { puts "ERR $r" } else { puts "OK <$r>" }
}
